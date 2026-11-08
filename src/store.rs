//! SQLite storage for projects, sessions and events.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS projects (
  container_id  TEXT PRIMARY KEY,
  name          TEXT,
  local_folder  TEXT,
  registered_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  session_id    TEXT PRIMARY KEY,
  container_id  TEXT,
  cwd           TEXT,
  title         TEXT,
  started_at    TEXT NOT NULL,
  ended_at      TEXT,
  last_event_at TEXT NOT NULL,
  last_event    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
  id            INTEGER PRIMARY KEY,
  session_id    TEXT NOT NULL,
  kind          TEXT NOT NULL,
  message       TEXT,
  created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_session ON events (session_id, id);
";

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub container_id: String,
    pub name: Option<String>,
    pub local_folder: Option<String>,
}

/// One hook event to record.
#[derive(Debug, Clone, Default)]
pub struct EventRecord<'a> {
    pub session_id: &'a str,
    pub container_id: Option<&'a str>,
    pub cwd: Option<&'a str>,
    pub kind: &'a str,
    pub message: Option<&'a str>,
    pub title: Option<&'a str>,
    pub ends_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionRow {
    pub project: Option<String>,
    pub container_id: Option<String>,
    pub local_folder: Option<String>,
    pub session_id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub last_event_at: String,
    pub last_event: String,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves the connection usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Inserts or updates a project. `None` keeps the stored value.
    pub fn upsert_project(
        &self,
        container_id: &str,
        name: Option<&str>,
        local_folder: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            &format!(
                "INSERT INTO projects (container_id, name, local_folder, registered_at)
                 VALUES (?1, ?2, ?3, {NOW})
                 ON CONFLICT (container_id) DO UPDATE SET
                   name = COALESCE(excluded.name, name),
                   local_folder = COALESCE(excluded.local_folder, local_folder)"
            ),
            params![container_id, name, local_folder],
        )?;
        Ok(())
    }

    /// Finds a project by full container ID or by a prefix such as `$HOSTNAME`.
    pub fn find_project(&self, id: &str) -> rusqlite::Result<Option<Project>> {
        self.conn()
            .query_row(
                "SELECT container_id, name, local_folder FROM projects
                 WHERE container_id = ?1 OR substr(container_id, 1, length(?1)) = ?1
                 ORDER BY container_id = ?1 DESC, registered_at DESC
                 LIMIT 1",
                [id],
                |row| {
                    Ok(Project {
                        container_id: row.get(0)?,
                        name: row.get(1)?,
                        local_folder: row.get(2)?,
                    })
                },
            )
            .optional()
    }

    pub fn session_title(&self, session_id: &str) -> rusqlite::Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT title FROM sessions WHERE session_id = ?1",
                [session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    pub fn record_event(&self, ev: &EventRecord) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            &format!(
                "INSERT INTO sessions
                   (session_id, container_id, cwd, title, started_at, ended_at, last_event_at, last_event)
                 VALUES (?1, ?2, ?3, ?4, {NOW}, CASE WHEN ?5 THEN {NOW} END, {NOW}, ?6)
                 ON CONFLICT (session_id) DO UPDATE SET
                   container_id = COALESCE(excluded.container_id, container_id),
                   cwd = COALESCE(excluded.cwd, cwd),
                   title = COALESCE(excluded.title, title),
                   ended_at = CASE WHEN ?5 THEN {NOW} ELSE ended_at END,
                   last_event_at = {NOW},
                   last_event = excluded.last_event"
            ),
            params![
                ev.session_id,
                ev.container_id,
                ev.cwd,
                ev.title,
                ev.ends_session,
                ev.kind
            ],
        )?;
        tx.execute(
            &format!(
                "INSERT INTO events (session_id, kind, message, created_at)
                 VALUES (?1, ?2, ?3, {NOW})"
            ),
            params![ev.session_id, ev.kind, ev.message],
        )?;
        tx.commit()
    }

    /// Sessions with their project, most recently active first.
    pub fn sessions(&self, limit: u32) -> rusqlite::Result<Vec<SessionRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT p.name, s.container_id, p.local_folder, s.session_id, s.title, s.cwd,
                    s.started_at, s.ended_at, s.last_event_at, s.last_event
             FROM sessions s LEFT JOIN projects p ON p.container_id = s.container_id
             ORDER BY s.last_event_at DESC, s.rowid DESC
             LIMIT ?1",
        )?;
        stmt.query_map([limit], |row| {
            Ok(SessionRow {
                project: row.get(0)?,
                container_id: row.get(1)?,
                local_folder: row.get(2)?,
                session_id: row.get(3)?,
                title: row.get(4)?,
                cwd: row.get(5)?,
                started_at: row.get(6)?,
                ended_at: row.get(7)?,
                last_event_at: row.get(8)?,
                last_event: row.get(9)?,
            })
        })?
        .collect()
    }

    #[cfg(test)]
    fn event_count(&self, session_id: &str) -> i64 {
        self.conn()
            .query_row(
                "SELECT count(*) FROM events WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "43afb2711be74f19612a17a789fdb3dd6ca1ded7737de4a8b1124035f87e41b2";

    #[test]
    fn upsert_keeps_existing_values() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_project(FULL, None, Some("/Users/me/src/api"))
            .unwrap();
        s.upsert_project(FULL, Some("api"), None).unwrap();
        let p = s.find_project(FULL).unwrap().unwrap();
        assert_eq!(p.name.as_deref(), Some("api"));
        assert_eq!(p.local_folder.as_deref(), Some("/Users/me/src/api"));
    }

    #[test]
    fn find_by_prefix() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_project(FULL, Some("api"), None).unwrap();
        assert_eq!(
            s.find_project(&FULL[..12]).unwrap().unwrap().container_id,
            FULL
        );
        assert!(s.find_project("ffff").unwrap().is_none());
        // LIKE wildcards in the input must not match anything.
        assert!(s.find_project("%").unwrap().is_none());
    }

    #[test]
    fn session_lifecycle() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_project(FULL, Some("api"), Some("/Users/me/src/api"))
            .unwrap();
        let base = EventRecord {
            session_id: "sess-1",
            container_id: Some(FULL),
            cwd: Some("/workspaces/api"),
            ..Default::default()
        };
        s.record_event(&EventRecord {
            kind: "SessionStart",
            title: Some("Fix login"),
            ..base.clone()
        })
        .unwrap();
        s.record_event(&EventRecord {
            kind: "Stop",
            message: Some("done"),
            ..base.clone()
        })
        .unwrap();

        assert_eq!(
            s.session_title("sess-1").unwrap().as_deref(),
            Some("Fix login")
        );
        let rows = s.sessions(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project.as_deref(), Some("api"));
        assert_eq!(rows[0].local_folder.as_deref(), Some("/Users/me/src/api"));
        assert_eq!(rows[0].last_event, "Stop");
        assert!(rows[0].ended_at.is_none());

        s.record_event(&EventRecord {
            kind: "SessionEnd",
            ends_session: true,
            ..base
        })
        .unwrap();
        let rows = s.sessions(10).unwrap();
        assert!(rows[0].ended_at.is_some());
        assert_eq!(rows[0].title.as_deref(), Some("Fix login"));
        assert_eq!(s.event_count("sess-1"), 3);
    }

    #[test]
    fn first_event_registers_session_without_session_start() {
        let s = Store::open_in_memory().unwrap();
        s.record_event(&EventRecord {
            session_id: "late",
            kind: "Notification",
            ..Default::default()
        })
        .unwrap();
        let rows = s.sessions(10).unwrap();
        assert_eq!(rows[0].session_id, "late");
        assert!(rows[0].project.is_none());
        assert!(s.session_title("late").unwrap().is_none());
        assert!(s.session_title("missing").unwrap().is_none());
    }

    #[test]
    fn reopening_a_file_keeps_data() {
        let dir = std::env::temp_dir().join(format!("watch-tower-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        let _ = std::fs::remove_file(&path);
        Store::open(&path)
            .unwrap()
            .upsert_project(FULL, Some("api"), None)
            .unwrap();
        assert!(
            Store::open(&path)
                .unwrap()
                .find_project(FULL)
                .unwrap()
                .is_some()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
