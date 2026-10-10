//! OpenCode: assistant messages in `opencode.db`, in `session_message`
//! (current versions) and `message` (earlier ones; a message kept in both has
//! the same id), or as JSON files in `storage/message/<session>/` (older
//! still). Each message counts once.

use crate::{Counts, Entry, files, model_name};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

pub const ID: &str = "opencode";

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    role: Option<String>,
    /// Earlier versions: `"modelID": "glm-5.3-flash"`.
    #[serde(rename = "modelID")]
    model_id: Option<String>,
    /// Current versions: `"model": {"id": "glm-5.3-flash", "providerID": …}`.
    model: Option<serde_json::Value>,
    tokens: Option<Tokens>,
    time: Option<Times>,
}

#[derive(Deserialize)]
struct Tokens {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    reasoning: u64,
    #[serde(default)]
    cache: Cache,
}

#[derive(Deserialize, Default)]
struct Cache {
    #[serde(default)]
    read: u64,
    #[serde(default)]
    write: u64,
}

impl Message {
    fn model(&self) -> &str {
        self.model_id
            .as_deref()
            .or_else(|| self.model.as_ref()?.get("id")?.as_str())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct Times {
    created: i64,
}

pub fn read(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        read_database(&dir.join("opencode.db"), &mut seen, &mut entries);
        for file in files(&dir.join("storage/message"), "json") {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Some(entry) = parse(&text, None, false, &mut seen) {
                entries.push(entry);
            }
        }
    }
    entries
}

fn read_database(path: &Path, seen: &mut HashSet<String>, entries: &mut Vec<Entry>) {
    if !path.is_file() {
        return;
    }
    let Ok(db) = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };
    let _ = db.busy_timeout(std::time::Duration::from_secs(2));
    // Newest first: a message in both tables is read from the current one.
    for (sql, assistant) in [
        (
            "SELECT id, data FROM session_message WHERE type = 'assistant'",
            true,
        ),
        ("SELECT id, data FROM message", false),
    ] {
        // A table this version doesn't have.
        let Ok(mut query) = db.prepare(sql) else {
            continue;
        };
        let Ok(rows) = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for (id, data) in rows.flatten() {
            if let Some(entry) = parse(&data, Some(id), assistant, seen) {
                entries.push(entry);
            }
        }
    }
}

/// `assistant`: the row is known to be one (current versions keep the role
/// in a column, not in the JSON).
fn parse(
    json: &str,
    id: Option<String>,
    assistant: bool,
    seen: &mut HashSet<String>,
) -> Option<Entry> {
    if !json.contains("\"tokens\"") {
        return None;
    }
    let message: Message = serde_json::from_str(json).ok()?;
    if !assistant && message.role.as_deref() != Some("assistant") {
        return None;
    }
    let model = model_name(message.model());
    let id = id.or(message.id)?;
    if !seen.insert(id) {
        return None;
    }
    let tokens = message.tokens?;
    let at = OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(message.time.as_ref()?.created) * 1_000_000,
    )
    .ok()?;
    Some(Entry {
        assistant: ID,
        model,
        at,
        counts: Counts {
            input: tokens.input,
            output: tokens.output + tokens.reasoning,
            cache_read: tokens.cache.read,
            cache_write: tokens.cache.write,
        },
        via_sdk: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGE: &str = r#"{"role":"assistant","modelID":"claude-opus-5-5","providerID":"anthropic","tokens":{"total":1,"input":2,"output":40,"reasoning":5,"cache":{"read":700,"write":11}},"time":{"created":1791039429424}}"#;

    #[test]
    fn reads_the_database_and_old_json_files_once_per_message() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let db = Connection::open(dir.path().join("opencode.db")).unwrap_or_else(|e| panic!("{e}"));
        db.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT)")
            .unwrap_or_else(|e| panic!("{e}"));
        let user = r#"{"role":"user","time":{"created":1}}"#;
        for (id, data) in [("msg_a", MESSAGE), ("msg_u", user)] {
            db.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, 's', ?2)",
                [id, data],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        drop(db);
        let legacy = dir.path().join("storage/message/ses_1");
        std::fs::create_dir_all(&legacy).unwrap_or_else(|e| panic!("{e}"));
        let with_id = |id: &str| MESSAGE.replacen('{', &format!(r#"{{"id":"{id}","#), 1);
        std::fs::write(legacy.join("msg_a.json"), with_id("msg_a"))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(legacy.join("msg_b.json"), with_id("msg_b"))
            .unwrap_or_else(|e| panic!("{e}"));

        let entries = read(&[dir.path().to_path_buf()]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].model, "claude-opus-5-5");
        assert_eq!(
            entries[0].counts,
            Counts {
                input: 2,
                output: 45,
                cache_read: 700,
                cache_write: 11
            }
        );
        assert_eq!(entries[0].at.date().to_string(), "2026-10-03");
    }

    #[test]
    fn reads_current_versions_and_counts_a_message_in_both_tables_once() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let db = Connection::open(dir.path().join("opencode.db")).unwrap_or_else(|e| panic!("{e}"));
        db.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT);
             CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER, data TEXT);",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let current = r#"{"time":{"created":1791039429424},"agent":"build","model":{"id":"glm-5.3-flash","providerID":"vultr-inference"},"finish":"stop","tokens":{"input":10,"output":20,"reasoning":3,"cache":{"read":100,"write":0}}}"#;
        let user = r#"{"time":{"created":1791039429424},"content":[],"tokens":{"input":1}}"#;
        for (id, kind, data) in [
            ("msg_new", "assistant", current),
            ("msg_both", "assistant", current),
            ("msg_user", "user", user),
        ] {
            db.execute(
                "INSERT INTO session_message (id, session_id, type, seq, data) VALUES (?1, 's', ?2, 0, ?3)",
                [id, kind, data],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        // Kept in the earlier table too, and one only there.
        for id in ["msg_both", "msg_old"] {
            db.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, 's', ?2)",
                [id, MESSAGE],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        drop(db);

        let entries = read(&[dir.path().to_path_buf()]);
        let models: Vec<&str> = entries.iter().map(|e| e.model.as_str()).collect();
        assert_eq!(
            models,
            ["glm-5.3-flash", "glm-5.3-flash", "claude-opus-5-5"]
        );
        assert_eq!(
            entries[0].counts,
            Counts {
                input: 10,
                output: 23,
                cache_read: 100,
                cache_write: 0
            }
        );
    }

    #[test]
    fn a_database_without_the_current_table_still_reads() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let db = Connection::open(dir.path().join("opencode.db")).unwrap_or_else(|e| panic!("{e}"));
        db.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT)")
            .unwrap_or_else(|e| panic!("{e}"));
        db.execute(
            "INSERT INTO message (id, session_id, data) VALUES ('m', 's', ?1)",
            [MESSAGE],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        drop(db);
        assert_eq!(read(&[dir.path().to_path_buf()]).len(), 1);
    }
}
