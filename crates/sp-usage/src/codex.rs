//! Codex: `sessions/YYYY/MM/DD/rollout-*.jsonl`. Each `token_count` event
//! carries the session's running total, and the same total is often sent
//! twice, so a request's usage is how much the total grew.

use crate::{Counts, Entry, files, model_name};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const ID: &str = "codex";

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    payload: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone, Copy, Default, PartialEq, Eq)]
struct Total {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

pub fn read(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for dir in dirs {
        for file in files(dir, "jsonl") {
            read_file(&file, &mut entries);
        }
    }
    entries
}

fn read_file(path: &Path, entries: &mut Vec<Entry>) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    let mut model = String::from("unknown");
    let mut previous = Total::default();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.contains("\"turn_context\"") && !line.contains("\"token_count\"") {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Line>(&line) else {
            continue;
        };
        let Some(payload) = line.payload else {
            continue;
        };
        if line.kind == "turn_context" {
            if let Some(name) = payload.get("model").and_then(|m| m.as_str()) {
                model = model_name(name);
            }
            continue;
        }
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        let Some(total) = payload
            .pointer("/info/total_token_usage")
            .and_then(|t| serde_json::from_value::<Total>(t.clone()).ok())
        else {
            continue;
        };
        let Some(at) = line
            .timestamp
            .as_deref()
            .and_then(|at| OffsetDateTime::parse(at, &Rfc3339).ok())
        else {
            continue;
        };
        // A smaller total means the session started counting again.
        let base = if total.input_tokens < previous.input_tokens {
            Total::default()
        } else {
            previous
        };
        previous = total;
        let input = total.input_tokens - base.input_tokens;
        let cached = total
            .cached_input_tokens
            .saturating_sub(base.cached_input_tokens)
            .min(input);
        entries.push(Entry {
            assistant: ID,
            model: model.clone(),
            at,
            counts: Counts {
                input: input - cached,
                output: total.output_tokens.saturating_sub(base.output_tokens),
                cache_read: cached,
                cache_write: 0,
            },
            via_sdk: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(input: u64, cached: u64, output: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-10-09T10:00:00.000Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"reasoning_output_tokens":0,"total_tokens":0}}}}}}}}"#
        )
    }

    #[test]
    fn counts_how_much_the_running_total_grew() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let day = dir.path().join("sessions/2026/10/09");
        std::fs::create_dir_all(&day).unwrap_or_else(|e| panic!("{e}"));
        let context = r#"{"timestamp":"2026-10-09T10:00:00.000Z","type":"turn_context","payload":{"model":"gpt-6-codex"}}"#;
        let empty = r#"{"timestamp":"2026-10-09T10:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":null}}"#;
        std::fs::write(
            day.join("rollout-1.jsonl"),
            [
                context,
                empty,
                &count(1000, 800, 50),
                &count(1000, 800, 50),
                &count(2500, 1800, 90),
            ]
            .join("\n"),
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let entries = read(&[dir.path().join("sessions")]);
        let counts: Vec<_> = entries.iter().map(|e| e.counts).collect();
        assert_eq!(
            counts,
            [
                Counts {
                    input: 200,
                    output: 50,
                    cache_read: 800,
                    cache_write: 0
                },
                Counts::default(),
                Counts {
                    input: 500,
                    output: 40,
                    cache_read: 1000,
                    cache_write: 0
                },
            ]
        );
        assert!(entries.iter().all(|e| e.model == "gpt-6-codex"));
    }
}
