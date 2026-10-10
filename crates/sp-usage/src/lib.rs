//! Tokens this computer's AI assistants used, counted from their own logs.
//!
//! Every request is counted once: assistants that log the same response more
//! than once are deduplicated by its id, and a request another app made
//! through the Claude Agent SDK (which logs it too) is counted only for the
//! app the user worked in. Counts are grouped per UTC day, assistant and
//! model; the server keeps the largest report per group, so sending the same
//! logs twice never doubles anything.

mod claude;
mod codex;
mod opencode;
mod source;

pub use source::source_id;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use time::{Date, OffsetDateTime};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Counts {
    /// Input not read from the prompt cache.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Counts {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn add(&mut self, other: &Counts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

/// One model response, as an assistant logged it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub assistant: &'static str,
    pub model: String,
    pub at: OffsetDateTime,
    pub counts: Counts,
    /// Made by another app through the Claude Agent SDK.
    pub via_sdk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayUsage {
    pub day: Date,
    pub assistant: String,
    pub model: String,
    pub counts: Counts,
}

/// Where each assistant keeps its logs.
#[derive(Debug, Clone, Default)]
pub struct Locations {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
    pub opencode: Vec<PathBuf>,
    /// Where to remember what was read, so the next scan only reads what
    /// changed. `None` reads everything.
    pub cache: Option<PathBuf>,
}

impl Locations {
    /// The default folders under `home`, and the ones environment variables point to.
    pub fn of(home: &Path) -> Self {
        let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
        let data = env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let mut claude = vec![home.join(".claude"), home.join(".config/claude")];
        if let Some(dir) = env("CLAUDE_CONFIG_DIR") {
            claude.insert(0, dir);
        }
        Self {
            claude: claude.into_iter().map(|dir| dir.join("projects")).collect(),
            codex: [env("CODEX_HOME").unwrap_or_else(|| home.join(".codex"))]
                .into_iter()
                .flat_map(|dir| [dir.join("sessions"), dir.join("archived_sessions")])
                .collect(),
            opencode: vec![data.join("opencode")],
            cache: None,
        }
    }
}

/// Reads every assistant's logs and returns their usage per day.
pub fn scan(locations: &Locations) -> Vec<DayUsage> {
    let mut entries = Vec::new();
    entries.extend(claude::read(&unique(&locations.claude)));
    entries.extend(codex::read(&unique(&locations.codex)));
    entries.extend(opencode::read(
        &unique(&locations.opencode),
        locations.cache.as_deref(),
    ));
    by_day(drop_sdk_echoes(entries))
}

fn unique(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    for dir in dirs {
        let real = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if !seen.contains(&real) {
            seen.push(real);
        }
    }
    seen
}

/// How far apart two logs of the same request can be.
const ECHO_WINDOW: i64 = 10 * 60;

/// An app that drives Claude through the Agent SDK logs each request
/// itself, and the SDK logs it again in Claude Code's folder. The SDK copy
/// goes when another assistant logged a request with the same input and
/// cache counts around the same time.
fn drop_sdk_echoes(entries: Vec<Entry>) -> Vec<Entry> {
    let fingerprint = |e: &Entry| (e.counts.input, e.counts.cache_read, e.counts.cache_write);
    let mut others: HashMap<(u64, u64, u64), Vec<i64>> = HashMap::new();
    for entry in entries.iter().filter(|e| e.assistant != claude::ID) {
        others
            .entry(fingerprint(entry))
            .or_default()
            .push(entry.at.unix_timestamp());
    }
    entries
        .into_iter()
        .filter(|entry| {
            !entry.via_sdk
                || !others.get(&fingerprint(entry)).is_some_and(|times| {
                    let at = entry.at.unix_timestamp();
                    times.iter().any(|t| (t - at).abs() <= ECHO_WINDOW)
                })
        })
        .collect()
}

fn by_day(entries: Vec<Entry>) -> Vec<DayUsage> {
    let mut days: BTreeMap<(Date, &'static str, String), Counts> = BTreeMap::new();
    for entry in entries {
        if entry.counts.total() == 0 {
            continue;
        }
        let day = entry.at.to_offset(time::UtcOffset::UTC).date();
        days.entry((day, entry.assistant, entry.model))
            .or_default()
            .add(&entry.counts);
    }
    days.into_iter()
        .map(|((day, assistant, model), counts)| DayUsage {
            day,
            assistant: assistant.to_string(),
            model,
            counts,
        })
        .collect()
}

/// Every file under `dir` with the extension `ext`, at any depth.
pub(crate) fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(listing) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in listing.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(kind) if kind.is_file() && path.extension().is_some_and(|e| e == ext) => {
                    found.push(path);
                }
                _ => {}
            }
        }
    }
    found
}

/// Model names as the server accepts them.
pub(crate) fn model_name(model: &str) -> String {
    let model = model.trim();
    let model = if model.is_empty() { "unknown" } else { model };
    model.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn entry(assistant: &'static str, at: OffsetDateTime, input: u64, via_sdk: bool) -> Entry {
        Entry {
            assistant,
            model: "m".into(),
            at,
            counts: Counts {
                input,
                output: 1,
                cache_read: 500,
                cache_write: 3,
            },
            via_sdk,
        }
    }

    #[test]
    fn drops_the_sdk_copy_of_a_request_another_assistant_logged() {
        let at = datetime!(2026-10-09 10:00 UTC);
        let kept = drop_sdk_echoes(vec![
            entry("opencode", at, 2, false),
            entry(claude::ID, at + time::Duration::seconds(3), 2, true),
            // Same counts, but Claude Code itself: a real request.
            entry(claude::ID, at, 2, false),
            // SDK request nobody else logged.
            entry(claude::ID, at, 9, true),
            // Same counts an hour later: another request.
            entry(claude::ID, at + time::Duration::hours(1), 2, true),
        ]);
        let left: Vec<_> = kept
            .iter()
            .map(|e| (e.assistant, e.counts.input, e.via_sdk))
            .collect();
        assert_eq!(
            left,
            [
                ("opencode", 2, false),
                (claude::ID, 2, false),
                (claude::ID, 9, true),
                (claude::ID, 2, true)
            ]
        );
    }

    #[test]
    fn groups_per_utc_day_assistant_and_model() {
        let rows = by_day(vec![
            entry("codex", datetime!(2026-10-09 23:30 -03:00), 1, false),
            entry("codex", datetime!(2026-10-10 01:00 UTC), 2, false),
            entry("opencode", datetime!(2026-10-10 05:00 UTC), 4, false),
        ]);
        let summary: Vec<_> = rows
            .iter()
            .map(|r| (r.day.to_string(), r.assistant.as_str(), r.counts.input))
            .collect();
        assert_eq!(
            summary,
            [
                ("2026-10-10".into(), "codex", 3),
                ("2026-10-10".into(), "opencode", 4)
            ]
        );
    }
}
