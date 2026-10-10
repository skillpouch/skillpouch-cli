//! OpenCode: assistant messages in `opencode.db`, in `session_message`
//! (current versions) and `message` (earlier ones; a message kept in both has
//! the same id), or as JSON files in `storage/message/<session>/` (older
//! still). Each message counts once.

use crate::{Counts, Entry, files, model_name};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

/// What one assistant message counted, kept between scans: current
/// databases hold gigabytes of conversation, and only messages changed since
/// the last scan are read again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Counted {
    model: String,
    at_ms: i64,
    counts: Counts,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Database {
    /// Per table: the newest `time_updated` read.
    read_up_to: HashMap<String, i64>,
    messages: HashMap<String, Counted>,
}

/// The scan cache: per database file.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ScanCache {
    format: u32,
    databases: HashMap<String, Database>,
}

const CACHE_FORMAT: u32 = 1;

impl ScanCache {
    fn load(path: Option<&Path>) -> Self {
        path.and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<ScanCache>(&bytes).ok())
            .filter(|cache| cache.format == CACHE_FORMAT)
            .unwrap_or_default()
    }

    fn save(&self, path: Option<&Path>) {
        let Some(path) = path else { return };
        let Ok(bytes) = serde_json::to_vec(self) else {
            return;
        };
        let staged = path.with_extension("tmp");
        if std::fs::write(&staged, bytes).is_ok() {
            let _ = std::fs::rename(&staged, path);
        }
    }
}

/// `cache`: where scans are remembered; `None` reads everything every time.
pub fn read(dirs: &[PathBuf], cache: Option<&Path>) -> Vec<Entry> {
    let mut remembered = ScanCache::load(cache);
    remembered.format = CACHE_FORMAT;
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        let path = dir.join("opencode.db");
        let database = remembered
            .databases
            .entry(path.to_string_lossy().into_owned())
            .or_default();
        read_database(&path, database);
        for (id, counted) in &database.messages {
            if seen.insert(id.clone())
                && let Some(entry) = entry(counted)
            {
                entries.push(entry);
            }
        }
        for file in files(&dir.join("storage/message"), "json") {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Some((id, counted)) = parse(&text, None, false)
                && seen.insert(id)
                && let Some(entry) = entry(&counted)
            {
                entries.push(entry);
            }
        }
    }
    remembered.save(cache);
    entries
}

fn entry(counted: &Counted) -> Option<Entry> {
    Some(Entry {
        assistant: ID,
        model: counted.model.clone(),
        at: OffsetDateTime::from_unix_timestamp_nanos(i128::from(counted.at_ms) * 1_000_000)
            .ok()?,
        counts: counted.counts,
        via_sdk: false,
    })
}

/// Reads what changed since the last scan into `database`.
fn read_database(path: &Path, database: &mut Database) {
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
    // Current table first: a message kept in both is read from it.
    for (table, filter, assistant) in [
        ("session_message", "type = 'assistant' AND ", true),
        ("message", "", false),
    ] {
        let from = database.read_up_to.get(table).copied().unwrap_or(i64::MIN);
        // `>=`: a message saved twice in the same millisecond isn't missed.
        let incremental =
            format!("SELECT id, data, time_updated FROM {table} WHERE {filter}time_updated >= ?1");
        // Versions without `time_updated` are read whole.
        let whole = format!("SELECT id, data, 0 FROM {table} WHERE {filter}?1 = ?1");
        // A table this version doesn't have.
        let Ok(mut query) = db.prepare(&incremental).or_else(|_| db.prepare(&whole)) else {
            continue;
        };
        let Ok(rows) = query.query_map([from], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) else {
            continue;
        };
        let mut newest = from;
        for (id, data, updated) in rows.flatten() {
            newest = newest.max(updated);
            if !assistant && database.messages.contains_key(&id) {
                continue;
            }
            if let Some((id, counted)) = parse(&data, Some(id), assistant) {
                database.messages.insert(id, counted);
            }
        }
        database.read_up_to.insert(table.to_string(), newest);
    }
}

/// `assistant`: the row is known to be one (current versions keep the role
/// in a column, not in the JSON).
fn parse(json: &str, id: Option<String>, assistant: bool) -> Option<(String, Counted)> {
    if !json.contains("\"tokens\"") {
        return None;
    }
    let message: Message = serde_json::from_str(json).ok()?;
    if !assistant && message.role.as_deref() != Some("assistant") {
        return None;
    }
    let model = model_name(message.model());
    let id = id.or(message.id)?;
    let at_ms = message.time.as_ref()?.created;
    let tokens = message.tokens?;
    Some((
        id,
        Counted {
            model,
            at_ms,
            counts: Counts {
                input: tokens.input,
                output: tokens.output + tokens.reasoning,
                cache_read: tokens.cache.read,
                cache_write: tokens.cache.write,
            },
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut entries: Vec<Entry>) -> Vec<Entry> {
        entries.sort_by(|a, b| (a.model.as_str(), a.at).cmp(&(b.model.as_str(), b.at)));
        entries
    }

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

        let entries = sorted(read(&[dir.path().to_path_buf()], None));
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

        let entries = sorted(read(&[dir.path().to_path_buf()], None));
        let models: Vec<&str> = entries.iter().map(|e| e.model.as_str()).collect();
        assert_eq!(
            models,
            ["claude-opus-5-5", "glm-5.3-flash", "glm-5.3-flash"]
        );
        assert_eq!(
            entries[1].counts,
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
        assert_eq!(read(&[dir.path().to_path_buf()], None).len(), 1);
    }

    #[test]
    fn later_scans_read_only_what_changed() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let cache = dir.path().join("cache.json");
        let db = Connection::open(dir.path().join("opencode.db")).unwrap_or_else(|e| panic!("{e}"));
        db.execute_batch(
            "CREATE TABLE session_message (id TEXT PRIMARY KEY, type TEXT, time_updated INTEGER, data TEXT)",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let reply = |output: u64| {
            format!(
                r#"{{"time":{{"created":1791039429424}},"model":{{"id":"glm-5.3-flash"}},"tokens":{{"input":1,"output":{output}}}}}"#
            )
        };
        let insert = |id: &str, updated: i64, output: u64| {
            db.execute(
                "INSERT OR REPLACE INTO session_message (id, type, time_updated, data) VALUES (?1, 'assistant', ?2, ?3)",
                rusqlite::params![id, updated, reply(output)],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        };
        insert("a", 10, 5);
        insert("b", 20, 7);
        let dirs = [dir.path().to_path_buf()];
        let total = |entries: Vec<Entry>| entries.iter().map(|e| e.counts.output).sum::<u64>();
        assert_eq!(total(read(&dirs, Some(&cache))), 12);

        // Rewritten behind the cache's back without a newer time: not read again.
        db.execute(
            "UPDATE session_message SET data = ?1 WHERE id = 'a'",
            [reply(500)],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(total(read(&dirs, Some(&cache))), 12);

        // Updated (a reply still streaming) and new ones are.
        insert("b", 30, 70);
        insert("c", 30, 1);
        assert_eq!(total(read(&dirs, Some(&cache))), 5 + 70 + 1);
        // Without the cache everything is read.
        assert_eq!(total(read(&dirs, None)), 500 + 70 + 1);
    }
}
