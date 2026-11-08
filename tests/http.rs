//! Starts the real binary on 127.0.0.1:0 with a fake osascript and docker.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-osascript.sh");
const FAKE_DOCKER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-docker.sh");
/// The container fake-docker.sh knows about.
const CONTAINER: &str = "aaaa000000000000000000000000000000000000000000000000000000000001";

struct Server {
    child: Child,
    addr: String,
    dir: PathBuf,
    out: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start() -> Server {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "server-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("osascript-argv");
    let db = dir.join("watch-tower.db");
    let mut child = Command::new(env!("CARGO_BIN_EXE_watch-tower"))
        .args([
            "serve",
            "-addr",
            "127.0.0.1:0",
            "-osascript",
            FAKE,
            "-default-title",
            "Default",
            "-docker",
            FAKE_DOCKER,
            "-db",
        ])
        .arg(&db)
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

    Server {
        child,
        addr,
        dir,
        out,
    }
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
        self.post_with(path, content_type, "", body)
    }

    fn post_with(&self, path: &str, content_type: &str, headers: &str, body: &str) -> Reply {
        self.raw(&format!(
            "POST {path} HTTP/1.1\r\nHost: host.docker.internal:4567\r\nConnection: close\r\n\
             {headers}Content-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
    }

    fn hook(&self, container: Option<&str>, body: &str) -> Reply {
        let headers = container
            .map(|c| format!("X-Container-Id: {c}\r\n"))
            .unwrap_or_default();
        self.post_with("/hooks/claude", "application/json", &headers, body)
    }

    /// Waits for the asynchronous notification sent after a hook.
    fn wait_notified(&self) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(args) = self.notified() {
                let _ = std::fs::remove_file(&self.out);
                return args;
            }
            assert!(Instant::now() < deadline, "no notification within 5s");
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn sessions(&self) -> Vec<Value> {
        let r = self.get("/sessions");
        assert_eq!(r.status, 200, "{}", r.body);
        assert!(
            r.head.contains("Content-Type: application/json"),
            "{}",
            r.head
        );
        serde_json::from_str::<Value>(&r.body)
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
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

#[test]
fn hook_flow_resolves_project_from_docker_label() {
    let s = start();
    let r = s.hook(
        Some("aaaa"),
        r#"{"session_id":"sess-0001-xyz","hook_event_name":"SessionStart","source":"startup",
            "session_title":"Fix login","cwd":"/workspaces/api"}"#,
    );
    assert_eq!((r.status, r.body.as_str()), (200, "ok"));

    let r = s.hook(
        Some("aaaa"),
        r#"{"session_id":"sess-0001-xyz","hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":"Done.\n\nAll tests pass."}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        s.wait_notified(),
        ["Done. All tests pass.", "api", "Fix login · Stop", "Ping"]
    );

    let sessions = s.sessions();
    assert_eq!(sessions.len(), 1);
    let row = &sessions[0];
    assert_eq!(row["project"], "api");
    assert_eq!(row["container_id"], CONTAINER);
    assert_eq!(row["local_folder"], "/Users/me/src/api");
    assert_eq!(row["session_id"], "sess-0001-xyz");
    assert_eq!(row["title"], "Fix login");
    assert_eq!(row["cwd"], "/workspaces/api");
    assert_eq!(row["last_event"], "Stop");
    assert!(row["ended_at"].is_null());

    let r = s.hook(
        Some("aaaa"),
        r#"{"session_id":"sess-0001-xyz","hook_event_name":"SessionEnd","reason":"prompt_input_exit"}"#,
    );
    assert_eq!(r.status, 200);
    let row = &s.sessions()[0];
    assert_eq!(row["last_event"], "SessionEnd");
    assert!(row["ended_at"].is_string());
}

#[test]
fn registered_name_wins_over_label() {
    let s = start();
    let r = s.post(
        "/projects",
        "application/json",
        &format!(r#"{{"container_id":"{CONTAINER}","name":"my-api"}}"#),
    );
    assert_eq!((r.status, r.body.as_str()), (200, "ok"));

    let r = s.hook(
        Some("aaaa00000000"),
        r#"{"session_id":"0123456789abcdef","hook_event_name":"Notification",
            "message":"Claude needs your permission","title":"Permission needed",
            "notification_type":"permission_prompt"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        s.wait_notified(),
        [
            "Claude needs your permission",
            "my-api",
            "01234567 · Permission needed",
            "Glass"
        ]
    );
    let row = &s.sessions()[0];
    assert_eq!(row["project"], "my-api");
    assert_eq!(row["local_folder"], "/Users/me/src/api");
}

#[test]
fn unknown_or_missing_container() {
    let s = start();
    let stop = r#"{"session_id":"s1","hook_event_name":"Stop"}"#;

    s.hook(None, stop);
    assert_eq!(s.wait_notified()[1], "Default");

    // fake-docker.sh does not know this container.
    s.hook(Some("bbbbccccdddd"), stop);
    assert_eq!(s.wait_notified()[1], "bbbbccccdddd");

    // Values that are not container IDs are ignored instead of reaching docker's argv.
    s.hook(Some("--help"), stop);
    assert_eq!(s.wait_notified()[1], "Default");
}

#[test]
fn hook_rejections() {
    let s = start();
    assert_eq!(s.hook(None, "").status, 400);
    assert_eq!(s.hook(None, "[]").status, 400);
    assert_eq!(s.hook(None, r#"{"hook_event_name":"Stop"}"#).status, 400);
    assert_eq!(
        s.post("/hooks/claude", "text/plain", r#"{"session_id":"s"}"#)
            .status,
        415
    );
    let r = s.get("/hooks/claude");
    assert_eq!(r.status, 405);
    assert!(r.head.contains("Allow: POST"), "{}", r.head);
    assert!(s.sessions().is_empty());
}

#[test]
fn project_rejections() {
    let s = start();
    for body in [
        r#"{"name":"x"}"#,
        r#"{"container_id":"not-hex"}"#,
        r#"{"container_id":""}"#,
        r#"{"container_id":1}"#,
    ] {
        assert_eq!(
            s.post("/projects", "application/json", body).status,
            400,
            "{body}"
        );
    }
    assert_eq!(s.get("/projects").status, 405);
}
