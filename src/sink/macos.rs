//! macOS notifications through osascript.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::{Notification, Sink};

/// Values arrive through `argv`, never interpolated into the script.
pub const SCRIPT: &str = r#"on run argv
  set {msg, ttl, sub, snd} to argv
  if sub is "" and snd is "" then
    display notification msg with title ttl
  else if snd is "" then
    display notification msg with title ttl subtitle sub
  else if sub is "" then
    display notification msg with title ttl sound name snd
  else
    display notification msg with title ttl subtitle sub sound name snd
  end if
end run"#;

pub struct Osascript {
    program: String,
    timeout: Duration,
}

impl Osascript {
    pub fn new(program: impl Into<String>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            timeout,
        }
    }
}

impl Sink for Osascript {
    fn send(&self, n: &Notification) -> Result<(), String> {
        let mut child = Command::new(&self.program)
            .arg("-e")
            .arg(SCRIPT)
            .args([&n.message, &n.title, &n.subtitle, &n.sound])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start {}: {e}", self.program))?;

        // Drain stderr on another thread so a chatty child cannot block on a full pipe.
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let stderr_reader = thread::spawn(move || {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            buf
        });

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} timed out after {:?}",
                        self.program, self.timeout
                    ));
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(format!("failed to wait for {}: {e}", self.program)),
            }
        };

        if status.success() {
            return Ok(());
        }
        let stderr = stderr_reader.join().unwrap_or_default();
        Err(format!(
            "{} exited with {status}: {}",
            self.program,
            stderr.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-osascript.sh");

    fn notification(message: &str) -> Notification {
        Notification {
            message: message.into(),
            title: "t".into(),
            subtitle: String::new(),
            sound: String::new(),
        }
    }

    #[test]
    fn success() {
        let sink = Osascript::new(FAKE, Duration::from_secs(5));
        assert_eq!(sink.send(&notification("hello")), Ok(()));
    }

    #[test]
    fn failure_reports_stderr() {
        let sink = Osascript::new(FAKE, Duration::from_secs(5));
        let err = sink.send(&notification("__fail__")).unwrap_err();
        assert!(err.contains("fake failure"), "{err}");
    }

    #[test]
    fn timeout_kills_the_child() {
        let sink = Osascript::new(FAKE, Duration::from_millis(200));
        let start = Instant::now();
        let err = sink.send(&notification("__hang__")).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn missing_program() {
        let sink = Osascript::new("/nonexistent/osascript", Duration::from_secs(1));
        assert!(sink.send(&notification("x")).is_err());
    }
}
