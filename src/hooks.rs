//! Claude Code hook events received on `/hooks/claude`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::sink::Notification;
use crate::store::EventRecord;

const MAX_BODY_CHARS: usize = 300;

/// The subset of the hook input watch-tower uses. Unknown fields are ignored.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct HookInput {
    pub session_id: String,
    pub hook_event_name: String,
    pub cwd: Option<String>,
    /// Notification
    pub message: Option<String>,
    /// Notification
    pub title: Option<String>,
    /// Notification
    pub notification_type: Option<String>,
    /// SessionStart
    pub session_title: Option<String>,
    /// Stop
    pub last_assistant_message: Option<String>,
}

impl HookInput {
    pub fn from_json(map: Map<String, Value>) -> Result<Self, String> {
        let input: Self = serde_json::from_value(Value::Object(map))
            .map_err(|e| format!("invalid hook input: {e}"))?;
        if input.session_id.is_empty() || input.session_id.len() > 256 {
            return Err("invalid session_id".into());
        }
        if input.hook_event_name.is_empty() || input.hook_event_name.len() > 64 {
            return Err("invalid hook_event_name".into());
        }
        Ok(input)
    }

    /// `transcript_title` is the latest title read from the transcript; it wins over
    /// `session_title`, which SessionStart only carries for resumed sessions.
    pub fn record<'a>(
        &'a self,
        container_id: Option<&'a str>,
        transcript_title: Option<&'a str>,
    ) -> EventRecord<'a> {
        EventRecord {
            session_id: &self.session_id,
            container_id,
            cwd: self.cwd.as_deref(),
            kind: &self.hook_event_name,
            message: self.raw_message(),
            title: transcript_title
                .or(self.session_title.as_deref())
                .filter(|t| !t.trim().is_empty()),
            ends_session: self.hook_event_name == "SessionEnd",
        }
    }

    fn raw_message(&self) -> Option<&str> {
        match self.hook_event_name.as_str() {
            "Notification" => self.message.as_deref(),
            "Stop" => self.last_assistant_message.as_deref(),
            _ => None,
        }
    }

    /// The notification body, or None when this event does not notify.
    fn body(&self) -> Option<String> {
        match self.hook_event_name.as_str() {
            "Notification" => Some(
                self.message
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| "Claude Code is waiting for you".into()),
            ),
            "Stop" => Some(
                self.last_assistant_message
                    .as_deref()
                    .map(summarize)
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "Claude Code finished responding".into()),
            ),
            _ => None,
        }
    }

    fn event_label(&self) -> String {
        match self.hook_event_name.as_str() {
            "Notification" => self
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "Notification".into()),
            other => other.to_string(),
        }
    }

    fn sound(&self) -> &'static str {
        match self.hook_event_name.as_str() {
            "Notification" => "Glass",
            _ => "Ping",
        }
    }

    /// Builds the notification to show, or None for events that only get recorded.
    pub fn notification(&self, project: &str, session_title: Option<&str>) -> Option<Notification> {
        let body = self.body()?;
        let session = session_title
            .filter(|t| !t.trim().is_empty())
            .map(|t| truncate(t, 40))
            .unwrap_or_else(|| short_id(&self.session_id).to_string());
        Some(Notification {
            title: truncate(project, 80),
            subtitle: format!("{session} · {}", truncate(&self.event_label(), 60)),
            message: body,
            sound: self.sound().to_string(),
        })
    }
}

/// Decodes the `X-Session-Title-Line` header: a base64-encoded transcript line such as
/// `{"type":"ai-title","aiTitle":"..."}` or `{"type":"custom-title","customTitle":"..."}`.
pub fn title_from_transcript_line(header: &str) -> Option<String> {
    let bytes = STANDARD.decode(header.trim()).ok()?;
    let line: Value = serde_json::from_slice(&bytes).ok()?;
    let title = line
        .get("customTitle")
        .or_else(|| line.get("aiTitle"))?
        .as_str()?
        .trim();
    (!title.is_empty()).then(|| truncate(title, 200))
}

pub fn short_id(id: &str) -> &str {
    match id.char_indices().nth(8) {
        Some((i, _)) => &id[..i],
        None => id,
    }
}

/// Collapses whitespace and cuts long assistant messages down to a notification body.
fn summarize(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&collapsed, MAX_BODY_CHARS)
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(json: &str) -> HookInput {
        HookInput::from_json(serde_json::from_str(json).unwrap()).unwrap()
    }

    #[test]
    fn parses_documented_notification_input() {
        let i = input(
            r#"{"session_id":"abc123","transcript_path":"/x.jsonl","cwd":"/workspaces/api",
                "hook_event_name":"Notification","message":"Claude needs your permission",
                "title":"Permission needed","notification_type":"permission_prompt",
                "effort":{"level":"high"}}"#,
        );
        let n = i.notification("api", None).unwrap();
        assert_eq!(n.title, "api");
        assert_eq!(n.subtitle, "abc123 · Permission needed");
        assert_eq!(n.message, "Claude needs your permission");
        assert_eq!(n.sound, "Glass");
    }

    #[test]
    fn stop_uses_last_assistant_message_and_session_title() {
        let i = input(
            r#"{"session_id":"0123456789abcdef","hook_event_name":"Stop",
                "last_assistant_message":"Done.\n\nAll   tests pass.","stop_hook_active":false}"#,
        );
        let n = i.notification("api", Some("Fix login")).unwrap();
        assert_eq!(n.subtitle, "Fix login · Stop");
        assert_eq!(n.message, "Done. All tests pass.");
        assert_eq!(n.sound, "Ping");

        let n = i.notification("api", None).unwrap();
        assert_eq!(n.subtitle, "01234567 · Stop");
    }

    #[test]
    fn fallbacks_when_fields_are_missing() {
        let n = input(r#"{"session_id":"s","hook_event_name":"Stop"}"#)
            .notification("p", None)
            .unwrap();
        assert_eq!(n.message, "Claude Code finished responding");
        let n = input(r#"{"session_id":"s","hook_event_name":"Notification","message":" "}"#)
            .notification("p", None)
            .unwrap();
        assert_eq!(n.message, "Claude Code is waiting for you");
        assert_eq!(n.subtitle, "s · Notification");
    }

    #[test]
    fn session_events_do_not_notify() {
        for name in ["SessionStart", "SessionEnd", "PreToolUse"] {
            let i = input(&format!(
                r#"{{"session_id":"s","hook_event_name":"{name}"}}"#
            ));
            assert!(i.notification("p", None).is_none(), "{name}");
        }
    }

    #[test]
    fn record_fields() {
        let start = input(
            r#"{"session_id":"s","hook_event_name":"SessionStart","source":"startup",
                "session_title":"Fix login","cwd":"/w"}"#,
        );
        let r = start.record(Some("abc"), None);
        assert_eq!(r.title, Some("Fix login"));
        assert_eq!(r.cwd, Some("/w"));
        assert_eq!(r.container_id, Some("abc"));
        assert!(!r.ends_session);
        assert_eq!(r.message, None);

        let end = input(r#"{"session_id":"s","hook_event_name":"SessionEnd","reason":"other"}"#);
        assert!(end.record(None, None).ends_session);

        let stop =
            input(r#"{"session_id":"s","hook_event_name":"Stop","last_assistant_message":"hi"}"#);
        assert_eq!(stop.record(None, None).message, Some("hi"));
    }

    #[test]
    fn transcript_title_wins() {
        let start =
            input(r#"{"session_id":"s","hook_event_name":"SessionStart","session_title":"old"}"#);
        assert_eq!(start.record(None, Some("new")).title, Some("new"));
        let stop = input(r#"{"session_id":"s","hook_event_name":"Stop"}"#);
        assert_eq!(stop.record(None, None).title, None);
    }

    #[test]
    fn title_header() {
        let enc = |s: &str| STANDARD.encode(s);
        assert_eq!(
            title_from_transcript_line(&enc(
                r#"{"type":"ai-title","aiTitle":"Rust-rewrite用のmise.toml","sessionId":"x"}"#
            ))
            .as_deref(),
            Some("Rust-rewrite用のmise.toml")
        );
        assert_eq!(
            title_from_transcript_line(&enc(
                r#"{"type":"custom-title","customTitle":" Fix login ","sessionId":"x"}"#
            ))
            .as_deref(),
            Some("Fix login")
        );
        for bad in [
            "",
            "!!!",
            &enc("not json"),
            &enc(r#"{"aiTitle":1}"#),
            &enc(r#"{"aiTitle":"  "}"#),
        ] {
            assert_eq!(title_from_transcript_line(bad), None, "{bad}");
        }
    }

    #[test]
    fn rejects_bad_input() {
        let bad = |json: &str| HookInput::from_json(serde_json::from_str(json).unwrap()).is_err();
        assert!(bad(r#"{"hook_event_name":"Stop"}"#));
        assert!(bad(r#"{"session_id":"s"}"#));
        assert!(bad(r#"{"session_id":"","hook_event_name":"Stop"}"#));
        assert!(bad(r#"{"session_id":1,"hook_event_name":"Stop"}"#));
        assert!(bad(
            r#"{"session_id":"s","hook_event_name":"Stop","cwd":["x"]}"#
        ));
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate("あいうえお", 3), "あいう…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(short_id("日本語のセッションID"), "日本語のセッショ");
        let long = "x ".repeat(400);
        assert_eq!(summarize(&long).chars().count(), MAX_BODY_CHARS + 1);
    }
}
