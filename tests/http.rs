//! Starts the real binary on 127.0.0.1:0 with a fake osascript.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-osascript.sh");

struct Server {
    child: Child,
    addr: String,
    out: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.out);
    }
}

fn start() -> Server {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "osascript-argv-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let mut child = Command::new(env!("CARGO_BIN_EXE_watch-tower"))
        .args([
            "serve",
            "-addr",
            "127.0.0.1:0",
            "-osascript",
            FAKE,
            "-default-title",
            "Default",
        ])
        .env("FAKE_OSASCRIPT_OUT", &out)
        .stderr(Stdio::piped())
        .spawn()
        .expect("start watch-tower");

    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let addr = loop {
        let line = lines
            .next()
            .expect("server exited before listening")
            .unwrap();
        if let Some(addr) = line.strip_prefix("watch-tower: listening on ") {
            break addr.to_string();
        }
    };
    // Keep draining the request log so the server never blocks on stderr.
    thread::spawn(move || lines.for_each(drop));

    Server { child, addr, out }
}

struct Reply {
    status: u16,
    head: String,
    body: String,
}

impl Server {
    fn raw(&self, request: &str) -> Reply {
        let mut stream = TcpStream::connect(&self.addr).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        Reply {
            status,
            head: head.to_string(),
            body: body.to_string(),
        }
    }

    fn get(&self, path: &str) -> Reply {
        self.raw(&format!(
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        ))
    }

    fn post(&self, path: &str, content_type: &str, body: &str) -> Reply {
        self.raw(&format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
             Content-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
    }

    /// The arguments after `-e <script>`, or None if osascript never ran.
    fn notified(&self) -> Option<Vec<String>> {
        let data = std::fs::read(&self.out).ok()?;
        let args: Vec<String> = data
            .split(|b| *b == 0)
            .map(|s| String::from_utf8(s.to_vec()).unwrap())
            .collect();
        // printf '%s\0' leaves an empty piece after the final NUL.
        assert_eq!(args.len(), 7, "{args:?}");
        assert_eq!(args[0], "-e");
        Some(args[2..6].to_vec())
    }
}

#[test]
fn healthz() {
    let s = start();
    let r = s.get("/healthz");
    assert_eq!((r.status, r.body.as_str()), (200, "ok"));

    let r = s.raw("HEAD /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    assert_eq!((r.status, r.body.as_str()), (200, ""));
}

#[test]
fn host_validation() {
    let s = start();
    for host in [
        "localhost:4567",
        "127.0.0.1",
        "[::1]:4567",
        "HOST.DOCKER.INTERNAL:4567",
    ] {
        let r = s.raw(&format!(
            "GET /healthz HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        ));
        assert_eq!(r.status, 200, "{host}");
    }
    for host in ["example.com", "evil.localhost", "192.168.5.2:4567"] {
        let r = s.raw(&format!(
            "GET /healthz HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        ));
        assert_eq!(r.status, 403, "{host}");
    }
    let r = s.raw("GET /healthz HTTP/1.0\r\n\r\n");
    assert_eq!(r.status, 403, "missing Host");
}

#[test]
fn not_found_and_method_not_allowed() {
    let s = start();
    assert_eq!(s.get("/nope").status, 404);

    let r = s.raw("DELETE /notify HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    assert_eq!(r.status, 405);
    assert!(r.head.contains("Allow: GET, POST"), "{}", r.head);

    let r = s.post("/healthz", "application/json", "{}");
    assert_eq!(r.status, 405);
    assert!(r.head.contains("Allow: GET, HEAD"), "{}", r.head);
}

#[test]
fn notify_from_query() {
    let s = start();
    let r = s.get("/notify?message=hello%20world&title=T&subtitle=S&sound=Ping");
    assert_eq!((r.status, r.body.as_str()), (200, "ok"));
    assert_eq!(s.notified().unwrap(), ["hello world", "T", "S", "Ping"]);
}

#[test]
fn notify_uses_default_title() {
    let s = start();
    assert_eq!(s.get("/notify?message=hi").status, 200);
    assert_eq!(s.notified().unwrap(), ["hi", "Default", "", ""]);
}

#[test]
fn notify_post_json_with_query_override() {
    let s = start();
    let r = s.post(
        "/notify?title=from-query",
        "application/json; charset=utf-8",
        r#"{"message":"from json \"quoted\" 'single' $(x)","title":"from-json","sound":"Glass","extra":1}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        s.notified().unwrap(),
        [
            "from json \"quoted\" 'single' $(x)",
            "from-query",
            "",
            "Glass"
        ]
    );
}

#[test]
fn notify_post_empty_body_uses_query() {
    let s = start();
    let r = s.post("/notify?message=q", "text/plain", "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(s.notified().unwrap(), ["q", "Default", "", ""]);
}

#[test]
fn notify_rejections() {
    let s = start();
    let cases = [
        ("text/plain", r#"{"message":"m"}"#.to_string(), 415),
        ("application/json", "[]".to_string(), 400),
        ("application/json", "{}{}".to_string(), 400),
        ("application/json", "{".to_string(), 400),
        ("application/json", r#"{"message":1}"#.to_string(), 400),
        (
            "application/json",
            r#"{"title":"no message"}"#.to_string(),
            400,
        ),
        (
            "application/json",
            format!(r#"{{"message":"{}"}}"#, "a".repeat(4097)),
            400,
        ),
        (
            "application/json",
            format!(r#"{{"message":"{}"}}"#, "a".repeat(64 * 1024)),
            413,
        ),
    ];
    for (ct, body, status) in cases {
        let r = s.post("/notify", ct, &body);
        assert_eq!(r.status, status, "{ct} {}", &body[..body.len().min(40)]);
    }
    assert!(s.notified().is_none(), "osascript must not run");
}

#[test]
fn notify_osascript_failure_is_500() {
    let s = start();
    assert_eq!(s.get("/notify?message=__fail__").status, 500);
}
