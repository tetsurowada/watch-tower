//! HTTP server: routing, request parsing and replies.

mod handlers;
mod host;

use std::sync::Arc;
use std::thread;
use std::time::Instant;

use serde_json::{Map, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::body::{self, BodyError, MAX_BODY};
use crate::docker::{self, ContainerLookup};
use crate::hooks;
use crate::sink::Sink;
use crate::store::Store;

pub struct App {
    pub sink: Box<dyn Sink>,
    pub store: Store,
    pub docker: Box<dyn ContainerLookup>,
    pub default_title: String,
    pub log_hooks: bool,
}

/// Serves requests forever, one thread per request.
pub fn run(server: Server, app: Arc<App>) {
    for request in server.incoming_requests() {
        let app = Arc::clone(&app);
        thread::spawn(move || handle(&app, request));
    }
}

pub struct Reply {
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
    if !host::allowed(header_value(request, "Host")) {
        return Reply::error(403, "forbidden");
    }
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    match (path, request.method()) {
        ("/healthz", Method::Get) => Reply::text(200, "ok"),
        ("/healthz", Method::Head) => Reply::text(200, ""),
        ("/healthz", _) => Reply::method_not_allowed("GET, HEAD"),

        ("/notify", Method::Get) => handlers::notify(app, query, None),
        ("/notify", Method::Post) => match read_body(request) {
            Ok(body) => handlers::notify(app, query, body),
            Err(reply) => reply,
        },
        ("/notify", _) => Reply::method_not_allowed("GET, POST"),

        ("/hooks/claude", Method::Post) => {
            let container_id = header_value(request, "X-Container-Id")
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| docker::is_container_id(v));
            let title = header_value(request, "X-Session-Title-Line")
                .and_then(hooks::title_from_transcript_line);
            match read_body(request) {
                Ok(Some(body)) => handlers::claude_hook(app, body, container_id, title),
                Ok(None) => Reply::error(400, "body is required"),
                Err(reply) => reply,
            }
        }
        ("/hooks/claude", _) => Reply::method_not_allowed("POST"),

        ("/sessions", Method::Get) => handlers::sessions(app),
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
