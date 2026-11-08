use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Map, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::body::{self, BodyError, MAX_BODY};
use crate::docker::{self, ContainerLookup};
use crate::hooks::HookInput;
use crate::notify_compat::Params;
use crate::sink::Sink;
use crate::store::{Project, Store};

const ALLOWED_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "host.docker.internal"];
const SESSIONS_LIMIT: u32 = 200;

pub struct App {
    pub sink: Box<dyn Sink>,
    pub store: Store,
    pub docker: Box<dyn ContainerLookup>,
    pub default_title: String,
    pub log_hooks: bool,
}

pub fn run(server: Server, app: Arc<App>) {
    for request in server.incoming_requests() {
        let app = Arc::clone(&app);
        thread::spawn(move || handle(&app, request));
    }
}

struct Reply {
    status: u16,
    body: String,
    content_type: &'static str,
    allow: Option<&'static str>,
}

impl Reply {
    fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            content_type: "text/plain; charset=utf-8",
            allow: None,
        }
    }

    fn json(body: String) -> Self {
        Self {
            content_type: "application/json",
            ..Self::text(200, body)
        }
    }

    fn error(status: u16, message: &str) -> Self {
        Self::text(status, format!("{message}\n"))
    }

    fn method_not_allowed(allow: &'static str) -> Self {
        Self {
            allow: Some(allow),
            ..Self::error(405, "method not allowed")
        }
    }

    fn from_body_error(e: BodyError) -> Self {
        match e {
            BodyError::UnsupportedMediaType => {
                Self::error(415, "content type must be application/json")
            }
            BodyError::TooLarge => Self::error(413, "body too large"),
            BodyError::Invalid(msg) => Self::error(400, msg),
        }
    }
}

fn handle(app: &Arc<App>, mut request: Request) {
    let start = Instant::now();
    let method = request.method().clone();
    let path = request.url().split('?').next().unwrap_or("").to_string();

    let reply = route(app, &mut request);

    eprintln!(
        "{method} {path} {} {:.1}ms",
        reply.status,
        start.elapsed().as_secs_f64() * 1000.0
    );

    let mut response = Response::from_string(reply.body).with_status_code(reply.status);
    response.add_header(header("Content-Type", reply.content_type));
    if let Some(allow) = reply.allow {
        response.add_header(header("Allow", allow));
    }
    if let Err(e) = request.respond(response) {
        eprintln!("failed to send response: {e}");
    }
}

fn route(app: &Arc<App>, request: &mut Request) -> Reply {
    if !host_allowed(header_value(request, "Host")) {
        return Reply::error(403, "forbidden");
    }
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    match (path, request.method()) {
        ("/healthz", Method::Get) => Reply::text(200, "ok"),
        ("/healthz", Method::Head) => Reply::text(200, ""),
        ("/healthz", _) => Reply::method_not_allowed("GET, HEAD"),

        ("/notify", Method::Get) => notify(app, query, None),
        ("/notify", Method::Post) => match read_body(request) {
            Ok(body) => notify(app, query, body),
            Err(reply) => reply,
        },
        ("/notify", _) => Reply::method_not_allowed("GET, POST"),

        ("/hooks/claude", Method::Post) => {
            let container_id = header_value(request, "X-Container-Id")
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| docker::is_container_id(v));
            match read_body(request) {
                Ok(Some(map)) => claude_hook(app, map, container_id),
                Ok(None) => Reply::error(400, "body is required"),
                Err(reply) => reply,
            }
        }
        ("/hooks/claude", _) => Reply::method_not_allowed("POST"),

        ("/projects", Method::Post) => match read_body(request) {
            Ok(Some(map)) => register_project(app, map),
            Ok(None) => Reply::error(400, "body is required"),
            Err(reply) => reply,
        },
        ("/projects", _) => Reply::method_not_allowed("POST"),

        ("/sessions", Method::Get) => sessions(app),
        ("/sessions", _) => Reply::method_not_allowed("GET"),

        _ => Reply::error(404, "not found"),
    }
}

/// Reads and validates a JSON object body. `Ok(None)` means the body was empty.
fn read_body(request: &mut Request) -> Result<Option<Map<String, Value>>, Reply> {
    let content_type = header_value(request, "Content-Type").map(str::to_string);
    let bytes = body::read_limited(request.as_reader(), MAX_BODY).map_err(|e| {
        eprintln!("failed to read body: {e}");
        Reply::error(400, "failed to read body")
    })?;
    body::parse_json_object(content_type.as_deref(), &bytes).map_err(Reply::from_body_error)
}

fn notify(app: &App, query: &str, body: Option<Map<String, Value>>) -> Reply {
    let json = match body.as_ref().map(Params::from_json) {
        None => Params::default(),
        Some(Ok(p)) => p,
        Some(Err(msg)) => return Reply::error(400, &msg),
    };
    let params = Params::merge(Params::from_query(query), json);
    let notification = match params.into_notification(&app.default_title) {
        Ok(n) => n,
        Err(msg) => return Reply::error(400, &msg),
    };
    match app.sink.send(&notification) {
        Ok(()) => Reply::text(200, "ok"),
        Err(e) => {
            eprintln!("notification failed: {e}");
            Reply::error(500, "notification failed")
        }
    }
}

fn claude_hook(app: &Arc<App>, map: Map<String, Value>, container_id: Option<String>) -> Reply {
    if app.log_hooks {
        eprintln!(
            "hook payload container={} {}",
            container_id.as_deref().unwrap_or("-"),
            Value::Object(map.clone())
        );
    }
    let input = match HookInput::from_json(map) {
        Ok(i) => i,
        Err(msg) => return Reply::error(400, &msg),
    };

    let project = container_id
        .as_deref()
        .and_then(|id| resolve_project(app, id));
    let stored_id = project
        .as_ref()
        .map(|p| p.container_id.clone())
        .or(container_id);
    if let Err(e) = app.store.record_event(&input.record(stored_id.as_deref())) {
        eprintln!("failed to record hook event: {e}");
    }

    let title = app
        .store
        .session_title(&input.session_id)
        .unwrap_or_else(|e| {
            eprintln!("failed to read session title: {e}");
            None
        });
    let project_name = project_name(project.as_ref(), stored_id.as_deref(), &app.default_title);
    if let Some(notification) = input.notification(&project_name, title.as_deref()) {
        // Answer the hook right away; osascript can take a while.
        let app = Arc::clone(app);
        thread::spawn(move || {
            if let Err(e) = app.sink.send(&notification) {
                eprintln!("notification failed: {e}");
            }
        });
    }
    Reply::text(200, "ok")
}

/// Looks the container up in the store, then asks docker and caches the answer.
fn resolve_project(app: &App, id: &str) -> Option<Project> {
    let stored = app.store.find_project(id).unwrap_or_else(|e| {
        eprintln!("failed to look up project {id}: {e}");
        None
    });
    // The folder is what a click opens, so fill it in even for named projects.
    if stored.as_ref().is_some_and(|p| p.local_folder.is_some()) {
        return stored;
    }
    let Some(info) = app.docker.inspect(id) else {
        return stored;
    };
    if let Err(e) = app
        .store
        .upsert_project(&info.id, None, info.local_folder.as_deref())
    {
        eprintln!("failed to store project {}: {e}", info.id);
    }
    app.store.find_project(&info.id).ok().flatten()
}

fn project_name(project: Option<&Project>, container_id: Option<&str>, default: &str) -> String {
    project
        .and_then(|p| {
            p.name
                .clone()
                .or_else(|| p.local_folder.as_deref().and_then(folder_name))
        })
        .or_else(|| container_id.map(|id| id.chars().take(12).collect()))
        .unwrap_or_else(|| default.to_string())
}

fn folder_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

#[derive(Deserialize)]
struct ProjectInput {
    container_id: String,
    name: Option<String>,
    local_folder: Option<String>,
}

fn register_project(app: &App, map: Map<String, Value>) -> Reply {
    let input: ProjectInput = match serde_json::from_value(Value::Object(map)) {
        Ok(i) => i,
        Err(e) => return Reply::error(400, &format!("invalid project: {e}")),
    };
    let id = input.container_id.trim().to_ascii_lowercase();
    if !docker::is_container_id(&id) {
        return Reply::error(400, "invalid container_id");
    }
    let name = input
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let folder = input.local_folder.as_deref().filter(|f| !f.is_empty());
    if name.is_some_and(|n| n.len() > 200) || folder.is_some_and(|f| f.len() > 4096) {
        return Reply::error(400, "name or local_folder is too long");
    }
    match app.store.upsert_project(&id, name, folder) {
        Ok(()) => Reply::text(200, "ok"),
        Err(e) => {
            eprintln!("failed to store project {id}: {e}");
            Reply::error(500, "failed to store project")
        }
    }
}

fn sessions(app: &App) -> Reply {
    let mut rows = match app.store.sessions(SESSIONS_LIMIT) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("failed to list sessions: {e}");
            return Reply::error(500, "failed to list sessions");
        }
    };
    for row in &mut rows {
        if row.project.is_none() {
            row.project = row.local_folder.as_deref().and_then(folder_name);
        }
    }
    match serde_json::to_string_pretty(&rows) {
        Ok(json) => Reply::json(json + "\n"),
        Err(e) => {
            eprintln!("failed to encode sessions: {e}");
            Reply::error(500, "failed to list sessions")
        }
    }
}

fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header is valid")
}

fn host_allowed(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        // [::1]:4567
        match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => return false,
        }
    } else if host.matches(':').count() == 1 {
        host.split_once(':').map_or(host, |(name, _)| name)
    } else {
        // Bare name, or a bare IPv6 address without a port.
        host
    };
    ALLOWED_HOSTS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        for ok in [
            "localhost",
            "localhost:4567",
            "LOCALHOST:4567",
            "127.0.0.1:4567",
            "[::1]:4567",
            "[::1]",
            "::1",
            "host.docker.internal:4567",
            "Host.Docker.Internal",
        ] {
            assert!(host_allowed(Some(ok)), "{ok}");
        }
        for ng in [
            "",
            "example.com",
            "example.com:4567",
            "localhost.example.com",
            "127.0.0.2:4567",
            "[::2]:4567",
            "[::1",
            "192.168.5.2:4567",
        ] {
            assert!(!host_allowed(Some(ng)), "{ng}");
        }
        assert!(!host_allowed(None));
    }

    #[test]
    fn project_names() {
        let p = |name: Option<&str>, folder: Option<&str>| Project {
            container_id: "c".into(),
            name: name.map(Into::into),
            local_folder: folder.map(Into::into),
        };
        assert_eq!(
            project_name(Some(&p(Some("api"), Some("/a/b"))), None, "d"),
            "api"
        );
        assert_eq!(
            project_name(Some(&p(None, Some("/Users/me/src/web"))), None, "d"),
            "web"
        );
        assert_eq!(
            project_name(None, Some("43afb2711be74f19612a"), "d"),
            "43afb2711be7"
        );
        assert_eq!(project_name(None, None, "Claude Code"), "Claude Code");
    }
}
