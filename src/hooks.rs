//! Claude Code hook events received on `/hooks/claude`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::sink::{Notification, State};
use crate::store::EventRecord;

const MAX_DETAIL_CHARS: usize = 80;

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

    /// What to say about this event, or None when it is only recorded.
    fn status(&self) -> Option<Status> {
        match self.hook_event_name.as_str() {
            "Stop" => Some(Status {
                label: "完了",
                state: Some(State::Done),
                detail: self
                    .last_assistant_message
                    .as_deref()
                    .and_then(first_sentence),
                sound: "Ping",
            }),
            "Notification" => Some(match self.notification_type.as_deref() {
                Some("permission_prompt") => Status {
                    label: "許可待ち",
                    state: Some(State::Wait),
                    detail: self.message.as_deref().and_then(permission_tool),
                    sound: "Glass",
                },
                Some("idle_prompt") => Status {
                    label: "入力待ち",
                    state: Some(State::Input),
                    detail: None,
                    sound: "Glass",
                },
                Some("elicitation_dialog") => Status {
                    label: "回答待ち",
                    state: Some(State::Input),
                    detail: None,
                    sound: "Glass",
                },
                _ => Status {
                    label: "通知",
                    state: None,
                    detail: self
                        .message
                        .as_deref()
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .map(|m| truncate(m, MAX_DETAIL_CHARS)),
                    sound: "Glass",
                },
            }),
            _ => None,
        }
    }

    /// Builds the notification to show, or None for events that only get recorded.
    ///
    /// Title: project. Subtitle: session. Body: state, then a short detail.
    pub fn notification(&self, project: &str, session_title: Option<&str>) -> Option<Notification> {
        let status = self.status()?;
        let session = session_title
            .filter(|t| !t.trim().is_empty())
            .map(|t| truncate(t, 60))
            .unwrap_or_else(|| short_id(&self.session_id).to_string());
        let message = match status.detail {
            Some(detail) => format!("{}\u{3000}{detail}", status.label),
            None => status.label.to_string(),
        };
        Some(Notification {
            title: truncate(project, 80),
            subtitle: session,
            message,
            sound: status.sound.to_string(),
            thread: self.session_id.clone(),
            state: status.state,
        })
    }
}

struct Status {
    label: &'static str,
    state: Option<State>,
    detail: Option<String>,
    sound: &'static str,
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

/// The first sentence of an assistant message, without Markdown decoration.
fn first_sentence(text: &str) -> Option<String> {
    let mut in_code = false;
    let line = text.lines().map(str::trim).find(|line| {
        if line.starts_with("```") {
            in_code = !in_code;
            return false;
        }
        !in_code && !line.is_empty() && !line.chars().all(|c| "-*_=|#".contains(c))
    })?;

    let mut line = line.trim_start_matches(['#', '>', ' ']);
    for bullet in ["- ", "* ", "+ "] {
        line = line.strip_prefix(bullet).unwrap_or(line);
    }
    if let Some((num, rest)) = line.split_once(". ")
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
    {
        line = rest;
    }
    let plain: String = line.replace("**", "").replace("__", "").replace('`', "");
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");

    let end = plain
        .char_indices()
        .find_map(|(i, c)| match c {
            '。' | '！' | '？' => Some(i + c.len_utf8()),
            '.' | '!' | '?' => {
                let next = plain[i + 1..].chars().next();
                (next.is_none() || next == Some(' ')).then_some(i + 1)
            }
            _ => None,
        })
        .unwrap_or(plain.len());
    let sentence = plain[..end].trim();
    (!sentence.is_empty()).then(|| truncate(sentence, MAX_DETAIL_CHARS))
}

/// "Claude needs your permission to use Bash" -> "Bash".
fn permission_tool(message: &str) -> Option<String> {
    let (_, tool) = message.split_once("permission to use ")?;
    let tool = tool.trim().trim_end_matches('.').trim();
    (!tool.is_empty()).then(|| truncate(tool, 40))
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
    fn permission_prompt() {
        let i = input(
            r#"{"session_id":"abc123","transcript_path":"/x.jsonl","cwd":"/workspaces/api",
                "hook_event_name":"Notification","message":"Claude needs your permission to use Bash",
                "title":"Permission needed","notification_type":"permission_prompt",
                "effort":{"level":"high"}}"#,
        );
        let n = i.notification("api", Some("Fix login")).unwrap();
        assert_eq!(n.title, "api");
        assert_eq!(n.subtitle, "Fix login");
        assert_eq!(n.message, "許可待ち\u{3000}Bash");
        assert_eq!(n.sound, "Glass");
        assert_eq!(n.thread, "abc123");
        assert_eq!(n.state, Some(State::Wait));
    }

    #[test]
    fn notification_types() {
        let n = |ty: &str, msg: &str| {
            input(&format!(
                r#"{{"session_id":"s","hook_event_name":"Notification","notification_type":"{ty}","message":"{msg}"}}"#
            ))
            .notification("p", None)
            .unwrap()
        };
        let idle = n("idle_prompt", "Claude is waiting for your input");
        assert_eq!(
            (idle.message.as_str(), idle.state),
            ("入力待ち", Some(State::Input))
        );
        let ask = n("elicitation_dialog", "Answer the question");
        assert_eq!(
            (ask.message.as_str(), ask.state),
            ("回答待ち", Some(State::Input))
        );
        let unknown_permission = n("permission_prompt", "Something else");
        assert_eq!(unknown_permission.message, "許可待ち");
        let other = n("auth_success", "Logged in");
        assert_eq!(
            (other.message.as_str(), other.state),
            ("通知\u{3000}Logged in", None)
        );
        let missing = input(r#"{"session_id":"s","hook_event_name":"Notification"}"#)
            .notification("p", None)
            .unwrap();
        assert_eq!(missing.message, "通知");
    }

    #[test]
    fn stop_uses_first_sentence() {
        let i = input(
            r##"{"session_id":"0123456789abcdef","hook_event_name":"Stop",
                "last_assistant_message":"# Done.\n\nAll   tests pass.","stop_hook_active":false}"##,
        );
        let n = i.notification("api", None).unwrap();
        assert_eq!(n.subtitle, "01234567");
        assert_eq!(n.message, "完了\u{3000}Done.");
        assert_eq!(n.sound, "Ping");
        assert_eq!(n.state, Some(State::Done));

        let bare = input(r#"{"session_id":"s","hook_event_name":"Stop"}"#)
            .notification("p", None)
            .unwrap();
        assert_eq!(bare.message, "完了");
    }

    #[test]
    fn first_sentences() {
        let f = |s: &str| first_sentence(s);
        assert_eq!(
            f("フックの設定はうまく動いています。ホストのサーバーに届いています。").as_deref(),
            Some("フックの設定はうまく動いています。")
        );
        assert_eq!(
            f("**`a0e9654`** をコミットしました").as_deref(),
            Some("a0e9654 をコミットしました")
        );
        assert_eq!(
            f("- 1 つ目の項目。\n- 2 つ目").as_deref(),
            Some("1 つ目の項目。")
        );
        assert_eq!(f("1. Build it. 2. Ship").as_deref(), Some("Build it."));
        assert_eq!(f("```sh\nls\n```\nDone!").as_deref(), Some("Done!"));
        assert_eq!(
            f("---\n\nVersion 1.2 is out").as_deref(),
            Some("Version 1.2 is out")
        );
        assert_eq!(f("   \n\n"), None);
        assert_eq!(
            f(&"あ".repeat(200)).unwrap().chars().count(),
            MAX_DETAIL_CHARS + 1
        );
    }

    #[test]
    fn permission_tools() {
        assert_eq!(
            permission_tool("Claude needs your permission to use Bash").as_deref(),
            Some("Bash")
        );
        assert_eq!(
            permission_tool("Claude needs your permission to use mcp__github__create_issue.")
                .as_deref(),
            Some("mcp__github__create_issue")
        );
        assert_eq!(permission_tool("Claude needs your permission"), None);
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
    }
}
