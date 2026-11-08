//! macOS notifications through osascript.

use std::process::Command;
use std::time::Duration;

use super::{Notification, Sink};
use crate::process;

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
        let mut command = Command::new(&self.program);
        command
            .arg("-e")
            .arg(SCRIPT)
            .args([&n.message, &n.title, &n.subtitle, &n.sound]);
        let out = process::run(command, self.timeout)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{} exited with {}: {}",
                self.program,
                out.status,
                out.stderr.trim()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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
}
