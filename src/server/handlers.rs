//! Endpoint handlers. Each takes parsed input and returns a reply.

use std::sync::Arc;
use std::thread;

use serde_json::{Map, Value};

use super::{App, Reply};
use crate::hooks::HookInput;
use crate::notify_compat::Params;
use crate::project;

const SESSIONS_LIMIT: u32 = 200;

/// `/notify`: the legacy endpoint used by notify.sh. Sends synchronously.
pub fn notify(app: &App, query: &str, body: Option<Map<String, Value>>) -> Reply {
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

/// `/hooks/claude`: records the event and notifies in the background.
pub fn claude_hook(
    app: &Arc<App>,
    body: Map<String, Value>,
    container_id: Option<String>,
    transcript_title: Option<String>,
) -> Reply {
    if app.log_hooks {
        eprintln!(
            "hook payload container={} {}",
            container_id.as_deref().unwrap_or("-"),
            Value::Object(body.clone())
        );
    }
    let input = match HookInput::from_json(body) {
        Ok(i) => i,
        Err(msg) => return Reply::error(400, &msg),
    };

    let project = container_id
        .as_deref()
        .and_then(|id| project::resolve(&app.store, app.docker.as_ref(), id));
    let container_id = project
        .as_ref()
        .map(|p| p.container_id.clone())
        .or(container_id);
    let record = input.record(container_id.as_deref(), transcript_title.as_deref());
    if let Err(e) = app.store.record_event(&record) {
        eprintln!("failed to record hook event: {e}");
    }

    let session_title = app
        .store
        .session_title(&input.session_id)
        .unwrap_or_else(|e| {
            eprintln!("failed to read session title: {e}");
            None
        });
    let name = project::display_name(
        project.as_ref(),
        container_id.as_deref(),
        &app.default_title,
    );
    if let Some(notification) = input.notification(&name, session_title.as_deref()) {
        // Answer the hook right away; sending can take a while.
        let app = Arc::clone(app);
        thread::spawn(move || {
            if let Err(e) = app.sink.send(&notification) {
                eprintln!("notification failed: {e}");
            }
        });
    }
    Reply::text(200, "ok")
}

/// `/sessions`: sessions with their project, most recently active first.
pub fn sessions(app: &App) -> Reply {
    let mut rows = match app.store.sessions(SESSIONS_LIMIT) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("failed to list sessions: {e}");
            return Reply::error(500, "failed to list sessions");
        }
    };
    for row in &mut rows {
        row.project = row.local_folder.as_deref().and_then(project::folder_name);
    }
    match serde_json::to_string_pretty(&rows) {
        Ok(json) => Reply::json(json + "\n"),
        Err(e) => {
            eprintln!("failed to encode sessions: {e}");
            Reply::error(500, "failed to list sessions")
        }
    }
}
