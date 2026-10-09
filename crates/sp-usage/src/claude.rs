//! Claude Code: one JSON object per line in `projects/<project>/<session>.jsonl`.
//! A response split into several content blocks is logged once per block, and
//! resumed sessions copy earlier lines into the new file, so responses are
//! counted once per message and request id.

use crate::{Counts, Entry, files, model_name};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const ID: &str = "claude-code";

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    entrypoint: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

pub fn read(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        for file in files(dir, "jsonl") {
            read_file(&file, &mut seen, &mut entries);
        }
    }
    entries
}

fn read_file(path: &Path, seen: &mut HashSet<String>, entries: &mut Vec<Entry>) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.contains("\"usage\"") {
            continue;
        }
        if let Some(entry) = parse(&line, seen) {
            entries.push(entry);
        }
    }
}

fn parse(line: &str, seen: &mut HashSet<String>) -> Option<Entry> {
    let line: Line = serde_json::from_str(line).ok()?;
    let message = line.message?;
    let usage = message.usage?;
    let model = message.model?;
    if model == "<synthetic>" {
        return None;
    }
    if let (Some(id), Some(request)) = (&message.id, &line.request_id)
        && !seen.insert(format!("{id}:{request}"))
    {
        return None;
    }
    let at = OffsetDateTime::parse(line.timestamp.as_deref()?, &Rfc3339).ok()?;
    Some(Entry {
        assistant: ID,
        model: model_name(&model),
        at,
        counts: Counts {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_read: usage.cache_read_input_tokens,
            cache_write: usage.cache_creation_input_tokens,
        },
        via_sdk: line
            .entrypoint
            .is_some_and(|entry| entry.starts_with("sdk")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = r#"{"type":"assistant","timestamp":"2026-10-09T10:00:00.000Z","requestId":"req_1","entrypoint":"cli","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":30,"cache_read_input_tokens":400,"output_tokens":50}}}"#;

    #[test]
    fn counts_a_response_once_however_often_it_is_logged() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let project = dir.path().join("projects/-home-ada");
        std::fs::create_dir_all(project.join("sub")).unwrap_or_else(|e| panic!("{e}"));
        let sdk = BLOCK
            .replace("msg_1", "msg_2")
            .replace("\"cli\"", "\"sdk-ts\"");
        std::fs::write(
            project.join("a.jsonl"),
            format!("{BLOCK}\n{BLOCK}\nnot json \"usage\"\n{sdk}\n"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let synthetic = BLOCK
            .replace("msg_1", "msg_3")
            .replace("claude-opus-5-5", "<synthetic>");
        std::fs::write(
            project.join("sub/b.jsonl"),
            format!("{BLOCK}\n{synthetic}\n"),
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let entries = read(&[dir.path().join("projects")]);
        assert_eq!(entries.len(), 2);
        let first = entries
            .iter()
            .find(|e| !e.via_sdk)
            .unwrap_or_else(|| panic!("missing"));
        assert_eq!(
            first.counts,
            Counts {
                input: 2,
                output: 50,
                cache_read: 400,
                cache_write: 30
            }
        );
        assert_eq!(first.model, "claude-opus-5-5");
        assert!(entries.iter().any(|e| e.via_sdk));
    }
}
