use std::sync::Arc;
use std::thread;
use std::time::Instant;

use tiny_http::{Header, Method, Request, Response, Server};

use crate::body::{self, BodyError, MAX_BODY};
use crate::notify_compat::Params;
use crate::sink::Sink;

const ALLOWED_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "host.docker.internal"];

pub struct App {
    pub sink: Box<dyn Sink>,
    pub default_title: String,
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
    allow: Option<&'static str>,
}

impl Reply {
    fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            allow: None,
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
}

fn handle(app: &App, mut request: Request) {
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
    response.add_header(header("Content-Type", "text/plain; charset=utf-8"));
    if let Some(allow) = reply.allow {
        response.add_header(header("Allow", allow));
    }
    if let Err(e) = request.respond(response) {
        eprintln!("failed to send response: {e}");
    }
}

fn route(app: &App, request: &mut Request) -> Reply {
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
        ("/notify", Method::Post) => {
            let content_type = header_value(request, "Content-Type").map(str::to_string);
            match body::read_limited(request.as_reader(), MAX_BODY) {
                Ok(bytes) => notify(app, query, Some((content_type.as_deref(), &bytes))),
                Err(e) => {
                    eprintln!("failed to read body: {e}");
                    Reply::error(400, "failed to read body")
                }
            }
        }
        ("/notify", _) => Reply::method_not_allowed("GET, POST"),
        _ => Reply::error(404, "not found"),
    }
}

fn notify(app: &App, query: &str, body: Option<(Option<&str>, &[u8])>) -> Reply {
    let json = match body.map(|(ct, bytes)| body::parse_json_object(ct, bytes)) {
        None | Some(Ok(None)) => Params::default(),
        Some(Ok(Some(map))) => match Params::from_json(&map) {
            Ok(p) => p,
            Err(msg) => return Reply::error(400, &msg),
        },
        Some(Err(BodyError::UnsupportedMediaType)) => {
            return Reply::error(415, "content type must be application/json");
        }
        Some(Err(BodyError::TooLarge)) => return Reply::error(413, "body too large"),
        Some(Err(BodyError::Invalid(msg))) => return Reply::error(400, msg),
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
    use super::host_allowed;

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
}
