//! OpenCode: assistant messages in `opencode.db` (current versions) or as
//! JSON files in `storage/message/<session>/` (older ones), counted once per
//! message id.

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
    #[serde(rename = "modelID")]
    model: Option<String>,
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
            if let Some(entry) = parse(&text, None, &mut seen) {
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
    let Ok(mut query) = db.prepare("SELECT id, data FROM message") else {
        return;
    };
    let Ok(rows) = query.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return;
    };
    for (id, data) in rows.flatten() {
        if let Some(entry) = parse(&data, Some(id), seen) {
            entries.push(entry);
        }
    }
}

fn parse(json: &str, id: Option<String>, seen: &mut HashSet<String>) -> Option<Entry> {
    if !json.contains("\"tokens\"") {
        return None;
    }
    let message: Message = serde_json::from_str(json).ok()?;
    if message.role.as_deref() != Some("assistant") {
        return None;
    }
    let id = id.or(message.id)?;
    if !seen.insert(id) {
        return None;
    }
    let tokens = message.tokens?;
    let at =
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(message.time?.created) * 1_000_000)
            .ok()?;
    Some(Entry {
        assistant: ID,
        model: model_name(message.model.as_deref().unwrap_or_default()),
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
}
