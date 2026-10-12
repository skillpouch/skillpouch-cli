//! Hooks: commands an assistant runs on its own events ("format the file
//! after every edit"). A pouch stores each one as `hook.json` in Claude
//! Code's terms; this module turns it into every assistant's own format and
//! back:
//!
//! ```text
//! { "name": "format", "event": "PostToolUse", "matcher": "Edit|Write",
//!   "command": "prettier --write \"$FILE\"", "timeout": 30 }
//! ```
//!
//! Each hook becomes one entry in the assistant's list for that event, so
//! SkillPouch can tell its entries from the user's and change only its own.

use crate::bundle::{ROOT_VARIABLE, is_part};
use crate::error::io;
use crate::{Result, SyncError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sp_harness::HookFormat;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// One hook, assistant-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// A Claude Code event: `PreToolUse`, `PostToolUse`, `Stop`, …
    pub event: String,
    /// Which tools (`Edit|Write`); empty or `*` for all.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub matcher: String,
    pub command: String,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl Hook {
    /// What it runs, for telling two hooks apart: the name and words don't count.
    pub fn same_action(&self, other: &Hook) -> bool {
        self.event == other.event
            && all(&self.matcher) == all(&other.matcher)
            && self.command.trim() == other.command.trim()
    }
}

/// `*`, `.*` and nothing all mean every tool.
fn all(matcher: &str) -> &str {
    match matcher.trim() {
        "*" | ".*" | "**" => "",
        other => other,
    }
}

pub fn parse_hook(bytes: &[u8]) -> Result<Hook> {
    let hook: Hook = serde_json::from_slice(bytes)
        .map_err(|error| SyncError::Invalid(format!("hook.json isn't valid: {error}")))?;
    if hook.command.trim().is_empty() || hook.event.trim().is_empty() {
        return Err(SyncError::Invalid(
            "hook.json needs an `event` and a `command`".into(),
        ));
    }
    Ok(hook)
}

/// `hook.json` as the pouch stores it.
pub fn hook_file(hook: &Hook) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(hook).unwrap_or_default();
    text.push('\n');
    text.into_bytes()
}

/// Claude Code event ↔ the assistant's name for it.
const GEMINI: &[(&str, &str)] = &[
    ("SessionStart", "SessionStart"),
    ("SessionEnd", "SessionEnd"),
    ("UserPromptSubmit", "BeforeAgent"),
    ("PreToolUse", "BeforeTool"),
    ("PostToolUse", "AfterTool"),
    ("Stop", "AfterAgent"),
    ("PreCompact", "PreCompress"),
    ("Notification", "Notification"),
];

const CURSOR: &[(&str, &str)] = &[
    ("SessionStart", "sessionStart"),
    ("SessionEnd", "sessionEnd"),
    ("UserPromptSubmit", "beforeSubmitPrompt"),
    ("PreToolUse", "preToolUse"),
    ("PostToolUse", "postToolUse"),
    ("PostToolUseFailure", "postToolUseFailure"),
    ("SubagentStart", "subagentStart"),
    ("SubagentStop", "subagentStop"),
    ("PreCompact", "preCompact"),
    ("Stop", "stop"),
];

/// Copilot CLI takes Claude's names as they are (and then Claude's tool
/// names in matchers and Claude's payloads), for these events.
const COPILOT: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "SubagentStart",
    "SubagentStop",
    "PreCompact",
    "Notification",
    "Stop",
];

/// Windsurf has an event per kind of tool: `pre_run_command`, `post_write_code`, …
const WINDSURF_TOOLS: &[(&str, &str)] = &[
    ("run_command", "Bash"),
    ("write_code", "Edit|Write"),
    ("read_code", "Read"),
    ("mcp_tool_use", "mcp__.*"),
];

fn windsurf_tool(token: &str) -> Option<&'static str> {
    match token {
        "Bash" => Some("run_command"),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Some("write_code"),
        "Read" => Some("read_code"),
        _ if token.starts_with("mcp__") => Some("mcp_tool_use"),
        _ => None,
    }
}

fn lookup(table: &[(&str, &'static str)], event: &str) -> Option<&'static str> {
    table.iter().find(|(ours, _)| *ours == event).map(|e| e.1)
}

fn reverse(table: &[(&'static str, &str)], native: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(_, theirs)| *theirs == native)
        .map(|e| e.0)
}

/// The assistant's events this hook runs on (none if it has no such event).
pub fn native_events(format: HookFormat, hook: &Hook) -> Vec<String> {
    let one = |name: Option<&str>| name.map(str::to_string).into_iter().collect();
    match format {
        HookFormat::Claude | HookFormat::ClaudeRoot => vec![hook.event.clone()],
        HookFormat::Gemini => one(lookup(GEMINI, &hook.event)),
        HookFormat::Cursor => one(lookup(CURSOR, &hook.event)),
        HookFormat::Copilot => one(COPILOT
            .contains(&hook.event.as_str())
            .then_some(&*hook.event)),
        HookFormat::Windsurf => {
            let stage = match hook.event.as_str() {
                "PreToolUse" => "pre",
                "PostToolUse" => "post",
                "UserPromptSubmit" => return vec!["pre_user_prompt".into()],
                "Stop" => return vec!["post_cascade_response".into()],
                _ => return Vec::new(),
            };
            let mut tools: Vec<&str> = if all(&hook.matcher).is_empty() {
                WINDSURF_TOOLS.iter().map(|(tool, _)| *tool).collect()
            } else {
                hook.matcher
                    .split('|')
                    .filter_map(|t| windsurf_tool(t.trim()))
                    .collect()
            };
            tools.dedup();
            tools
                .into_iter()
                .map(|tool| format!("{stage}_{tool}"))
                .collect()
        }
    }
}

/// The entry for `hook` in the assistant's list for one event.
pub fn to_native(format: HookFormat, hook: &Hook) -> Value {
    let matcher = all(&hook.matcher);
    let mut handler = json!({ "type": "command", "command": hook.command });
    match format {
        HookFormat::Claude | HookFormat::ClaudeRoot | HookFormat::Gemini => {
            if format == HookFormat::Gemini {
                handler["name"] = json!(hook.name);
                if let Some(timeout) = hook.timeout {
                    handler["timeout"] = json!(timeout * 1000);
                }
            } else if let Some(timeout) = hook.timeout {
                handler["timeout"] = json!(timeout);
            }
            let mut group = json!({ "hooks": [handler] });
            if !matcher.is_empty() {
                group["matcher"] = json!(matcher);
            }
            group
        }
        HookFormat::Cursor => {
            let mut entry = json!({ "command": hook.command });
            if !matcher.is_empty() {
                entry["matcher"] = json!(matcher);
            }
            entry
        }
        HookFormat::Copilot => {
            if !matcher.is_empty() {
                handler["matcher"] = json!(matcher);
            }
            if let Some(timeout) = hook.timeout {
                handler["timeoutSec"] = json!(timeout);
            }
            handler
        }
        HookFormat::Windsurf => json!({ "command": hook.command }),
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The hooks in one entry of an assistant's list (names left empty).
pub fn from_native(format: HookFormat, event: &str, entry: &Value) -> Vec<Hook> {
    let hook = |event: &str, matcher: String, command: String, timeout: Option<u64>| Hook {
        name: String::new(),
        description: String::new(),
        event: event.to_string(),
        matcher,
        command,
        timeout,
    };
    match format {
        HookFormat::Claude | HookFormat::ClaudeRoot | HookFormat::Gemini => {
            let canonical = if format == HookFormat::Gemini {
                reverse(GEMINI, event)
            } else {
                Some(event)
            };
            let Some(canonical) = canonical else {
                return Vec::new();
            };
            let matcher = text(entry, "matcher");
            entry
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|handler| {
                    handler
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("command")
                        == "command"
                })
                .filter_map(|handler| {
                    let command = text(handler, "command");
                    let timeout = handler.get("timeout").and_then(Value::as_u64).map(|t| {
                        if format == HookFormat::Gemini {
                            t.div_ceil(1000)
                        } else {
                            t
                        }
                    });
                    (!command.is_empty())
                        .then(|| hook(canonical, matcher.clone(), command, timeout))
                })
                .collect()
        }
        HookFormat::Cursor => {
            let command = text(entry, "command");
            match reverse(CURSOR, event) {
                Some(canonical) if !command.is_empty() => {
                    vec![hook(canonical, text(entry, "matcher"), command, None)]
                }
                _ => Vec::new(),
            }
        }
        HookFormat::Copilot => {
            let command = ["command", "bash"]
                .iter()
                .map(|key| text(entry, key))
                .find(|command| !command.is_empty())
                .unwrap_or_default();
            match COPILOT.iter().find(|name| **name == event) {
                Some(canonical) if !command.is_empty() => vec![hook(
                    canonical,
                    text(entry, "matcher"),
                    command,
                    entry.get("timeoutSec").and_then(Value::as_u64),
                )],
                _ => Vec::new(),
            }
        }
        HookFormat::Windsurf => {
            let command = text(entry, "command");
            if command.is_empty() {
                return Vec::new();
            }
            let (canonical, matcher) = match event {
                "pre_user_prompt" => ("UserPromptSubmit", ""),
                "post_cascade_response" => ("Stop", ""),
                _ => {
                    let Some((stage, tool)) = event.split_once('_') else {
                        return Vec::new();
                    };
                    let Some(matcher) = lookup(WINDSURF_TOOLS, tool) else {
                        return Vec::new();
                    };
                    match stage {
                        "pre" => ("PreToolUse", matcher),
                        "post" => ("PostToolUse", matcher),
                        _ => return Vec::new(),
                    }
                }
            };
            vec![hook(canonical, matcher.to_string(), command, None)]
        }
    }
}

/// Where the event lists are: under `hooks`, or at the top (Factory `hooks.json`).
fn events_path(format: HookFormat) -> &'static [&'static str] {
    match format {
        HookFormat::ClaudeRoot => &[],
        _ => &["hooks"],
    }
}

/// Every event list in the file, as it is now.
pub fn read(path: &Path, format: HookFormat) -> BTreeMap<String, Vec<Value>> {
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&sp_harness::strip_jsonc(&text)) else {
        return BTreeMap::new();
    };
    let mut events = &value;
    for key in events_path(format) {
        match events.get(key) {
            Some(inner) => events = inner,
            None => return BTreeMap::new(),
        }
    }
    events
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(event, list)| Some((event.clone(), list.as_array()?.clone())))
        .collect()
}

/// One entry SkillPouch put in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrittenHook {
    pub name: String,
    pub event: String,
    pub entry: Value,
}

/// What SkillPouch did to one hooks file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookFile {
    pub written: Vec<WrittenHook>,
    /// Every event list right after writing: an entry that wasn't there
    /// then was added by hand since.
    #[serde(default)]
    pub seen: BTreeMap<String, Vec<Value>>,
}

/// What changed in a file since SkillPouch last wrote it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    /// SkillPouch's hooks edited by hand: `(name, hook as it is now)`.
    pub edited: Vec<(String, Hook)>,
    /// Hooks someone added that SkillPouch didn't write.
    pub added: Vec<Added>,
}

/// A hook found in a file, and the entry it came from when that entry
/// holds only this hook (so SkillPouch can take the entry over).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub hook: Hook,
    pub entry: Option<(String, Value)>,
}

/// Compares the file with what SkillPouch left there. Without a record of
/// that, every hook in the file counts as added.
pub fn changes(path: &Path, format: HookFormat, previous: Option<&HookFile>) -> Changes {
    let mut found = Changes::default();
    for (event, list) in read(path, format) {
        let seen = previous.and_then(|file| file.seen.get(&event));
        let mut missing: Vec<&WrittenHook> = previous
            .into_iter()
            .flat_map(|file| &file.written)
            .filter(|written| written.event == event && !list.contains(&written.entry))
            .collect();
        for entry in list
            .iter()
            .filter(|entry| !seen.is_some_and(|seen| seen.contains(entry)))
        {
            let hooks = from_native(format, &event, entry);
            match (missing.is_empty(), hooks.into_iter().next()) {
                // An entry of ours went and this one came: it was edited.
                (false, Some(hook)) => {
                    let ours = missing.remove(0);
                    found.edited.push((ours.name.clone(), hook));
                }
                (true, Some(_)) => {
                    // Claude's groups may hold several commands.
                    let hooks = from_native(format, &event, entry);
                    let single = hooks.len() == 1;
                    found.added.extend(hooks.into_iter().map(|hook| Added {
                        hook,
                        entry: single.then(|| (event.clone(), entry.clone())),
                    }));
                }
                (_, None) => {}
            }
        }
    }
    found
}

/// Puts `hooks` into the file in place of what SkillPouch put there before.
/// `Err` (inside `Ok`) says why the file was left alone.
pub fn write(
    path: &Path,
    format: HookFormat,
    hooks: &[Hook],
    previous: Option<&HookFile>,
) -> Result<std::result::Result<HookFile, String>> {
    let mut wanted: BTreeMap<String, Vec<WrittenHook>> = BTreeMap::new();
    for hook in hooks {
        for event in native_events(format, hook) {
            wanted.entry(event.clone()).or_default().push(WrittenHook {
                name: hook.name.clone(),
                event,
                entry: to_native(format, hook),
            });
        }
    }
    if format == HookFormat::Copilot {
        return write_own(path, wanted).map(Ok);
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io(path)(error)),
    };
    let current = read(path, format);
    let mut events: Vec<String> = wanted.keys().cloned().collect();
    events.extend(
        previous
            .into_iter()
            .flat_map(|file| file.written.iter().map(|w| w.event.clone())),
    );
    events.sort();
    events.dedup();

    let mut updated = text.clone();
    let mut record = HookFile::default();
    for event in &events {
        let mut list = current.get(event).cloned().unwrap_or_default();
        // A plugin's hook edited in place goes back to what the plugin says:
        // kept, the edit would run beside it and outlive the plugin. An edit
        // keeps the `CLAUDE_PLUGIN_ROOT` it was given; the user's own hooks,
        // new or edited, don't have it and stay.
        let mut edited = previous
            .into_iter()
            .flat_map(|file| &file.written)
            .filter(|w| &w.event == event && is_part(&w.name) && !list.contains(&w.entry))
            .count();
        let seen = previous.and_then(|file| file.seen.get(event));
        list.retain(|entry| {
            let new = !seen.is_some_and(|seen| seen.contains(entry));
            let drop = edited > 0
                && new
                && from_native(format, event, entry)
                    .iter()
                    .any(|hook| hook.command.contains(ROOT_VARIABLE));
            edited -= usize::from(drop);
            !drop
        });
        for old in previous
            .into_iter()
            .flat_map(|file| &file.written)
            .filter(|w| &w.event == event)
        {
            if let Some(at) = list.iter().position(|entry| entry == &old.entry) {
                list.remove(at);
            }
        }
        // A hook the user already has there isn't added a second time.
        let theirs: Vec<Hook> = list
            .iter()
            .flat_map(|entry| from_native(format, event, entry))
            .collect();
        let ours: Vec<WrittenHook> = wanted
            .remove(event)
            .unwrap_or_default()
            .into_iter()
            .filter(|written| {
                let hook = from_native(format, event, &written.entry);
                !hook
                    .iter()
                    .any(|ours| theirs.iter().any(|t| t.same_action(ours)))
            })
            .collect();
        list.extend(ours.iter().map(|w| w.entry.clone()));
        let mut at: Vec<&str> = events_path(format).to_vec();
        at.push(event);
        let value =
            (!list.is_empty() || current.contains_key(event)).then(|| Value::Array(list.clone()));
        // An empty list we emptied ourselves goes away again.
        let value = value.filter(|value| {
            !(value.as_array().is_some_and(Vec::is_empty)
                && previous.is_some_and(|file| file.written.iter().any(|w| &w.event == event)))
        });
        updated = match crate::jsonc::set_value(&updated, &at, value.as_ref()) {
            Ok(updated) => updated,
            Err(reason) => return Ok(Err(format!("{}: {reason}", path.display()))),
        };
        record.written.extend(ours);
        if !list.is_empty() {
            record.seen.insert(event.clone(), list);
        }
    }
    for (event, list) in current {
        record.seen.entry(event).or_insert(list);
    }
    if format == HookFormat::Cursor
        && !record.written.is_empty()
        && !updated.contains("\"version\"")
    {
        updated =
            crate::jsonc::set_value(&updated, &["version"], Some(&json!(1))).unwrap_or(updated);
    }
    if updated != text {
        crate::item::write_file(path, &crate::item::File::new(updated.into_bytes()))?;
    }
    Ok(Ok(record))
}

/// A file only SkillPouch writes (Copilot's hooks folder): replaced whole,
/// removed when there's nothing to put in it.
fn write_own(path: &Path, wanted: BTreeMap<String, Vec<WrittenHook>>) -> Result<HookFile> {
    if wanted.is_empty() {
        match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(io(path)(error));
            }
            _ => return Ok(HookFile::default()),
        }
    }
    let lists: serde_json::Map<String, Value> = wanted
        .iter()
        .map(|(event, ours)| {
            (
                event.clone(),
                Value::Array(ours.iter().map(|w| w.entry.clone()).collect()),
            )
        })
        .collect();
    let mut text =
        serde_json::to_string_pretty(&json!({ "version": 1, "hooks": lists })).unwrap_or_default();
    text.push('\n');
    if fs::read_to_string(path).ok().as_deref() != Some(text.as_str()) {
        crate::item::write_file(path, &crate::item::File::new(text.into_bytes()))?;
    }
    let seen = wanted
        .iter()
        .map(|(event, ours)| {
            (
                event.clone(),
                ours.iter().map(|w| w.entry.clone()).collect(),
            )
        })
        .collect();
    Ok(HookFile {
        written: wanted.into_values().flatten().collect(),
        seen,
    })
}

/// A name for a hook found in an assistant: `posttooluse-prettier`.
pub fn name_for(hook: &Hook) -> String {
    let program = hook
        .command
        .split_whitespace()
        .find(|word| !word.contains('='))
        .unwrap_or("hook");
    let program = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let program = program.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    crate::item::safe_name(&format!("{}-{program}", hook.event.to_lowercase()))
        .unwrap_or_else(|| "hook".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format_hook() -> Hook {
        Hook {
            name: "format".into(),
            description: "Format edited files".into(),
            event: "PostToolUse".into(),
            matcher: "Edit|Write".into(),
            command: "prettier --write .".into(),
            timeout: Some(30),
        }
    }

    fn strip(hook: &Hook) -> Hook {
        Hook {
            name: String::new(),
            description: String::new(),
            ..hook.clone()
        }
    }

    #[test]
    fn every_format_round_trips() {
        let hook = format_hook();
        for format in [
            HookFormat::Claude,
            HookFormat::ClaudeRoot,
            HookFormat::Gemini,
            HookFormat::Copilot,
        ] {
            let events = native_events(format, &hook);
            assert_eq!(events.len(), 1, "{format:?}");
            let back = from_native(format, &events[0], &to_native(format, &hook));
            assert_eq!(back, vec![strip(&hook)], "{format:?}");
        }
        assert_eq!(native_events(HookFormat::Gemini, &hook), ["AfterTool"]);
        assert_eq!(
            to_native(HookFormat::Gemini, &hook)["hooks"][0]["timeout"],
            30_000
        );
        // Cursor and Windsurf have no timeouts.
        let cursor = from_native(
            HookFormat::Cursor,
            "postToolUse",
            &to_native(HookFormat::Cursor, &hook),
        );
        assert_eq!(cursor[0].command, hook.command);
        assert_eq!(
            native_events(HookFormat::Windsurf, &hook),
            ["post_write_code"]
        );
        let every_tool = Hook {
            matcher: "*".into(),
            ..hook.clone()
        };
        assert_eq!(native_events(HookFormat::Windsurf, &every_tool).len(), 4);
        let windsurf = from_native(
            HookFormat::Windsurf,
            "post_write_code",
            &json!({"command": "x"}),
        );
        assert_eq!(
            (windsurf[0].event.as_str(), windsurf[0].matcher.as_str()),
            ("PostToolUse", "Edit|Write")
        );
        // No such event there: left out, not guessed.
        let teammate = Hook {
            event: "TeammateIdle".into(),
            ..hook
        };
        assert!(native_events(HookFormat::Gemini, &teammate).is_empty());
    }

    #[test]
    fn writes_only_its_own_entries_and_keeps_comments() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            "{\n  // mine\n  \"hooks\": {\n    \"PostToolUse\": [\n      { \"hooks\": [{ \"type\": \"command\", \"command\": \"echo mine\" }] }\n    ]\n  }\n}\n",
        )
        .map_err(io(&path))?;
        let hook = format_hook();
        let first = write(&path, HookFormat::Claude, std::slice::from_ref(&hook), None)?
            .map_err(SyncError::Invalid)?;
        let text = fs::read_to_string(&path).map_err(io(&path))?;
        assert!(text.contains("// mine"), "{text}");
        assert_eq!(read(&path, HookFormat::Claude)["PostToolUse"].len(), 2);
        // Nothing new since: no changes.
        assert_eq!(
            changes(&path, HookFormat::Claude, Some(&first)),
            Changes::default()
        );

        // Changed in the pouch: ours is replaced, the user's stays.
        let changed = Hook {
            command: "biome format".into(),
            ..hook.clone()
        };
        let second = write(&path, HookFormat::Claude, &[changed], Some(&first))?
            .map_err(SyncError::Invalid)?;
        let list = &read(&path, HookFormat::Claude)["PostToolUse"];
        assert_eq!(list.len(), 2);
        assert_eq!(list[1]["hooks"][0]["command"], "biome format");

        // Gone from the pouch: ours goes, the user's stays.
        write(&path, HookFormat::Claude, &[], Some(&second))?.map_err(SyncError::Invalid)?;
        let list = &read(&path, HookFormat::Claude)["PostToolUse"];
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["hooks"][0]["command"], "echo mine");
        Ok(())
    }

    #[test]
    fn hand_edits_and_new_hooks_are_found() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        // Before SkillPouch wrote anything, the user's hooks count as added.
        fs::write(&path, r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"},{"type":"command","command":"notify"}]}]}}"#)
            .map_err(io(&path))?;
        let found = changes(&path, HookFormat::Claude, None);
        assert_eq!(found.added.len(), 2);
        assert_eq!(name_for(&found.added[0].hook), "stop-say");
        // Two commands in one entry: neither can take the entry over.
        assert!(found.added.iter().all(|added| added.entry.is_none()));

        let record = write(&path, HookFormat::Claude, &[format_hook()], None)?
            .map_err(SyncError::Invalid)?;
        let text = fs::read_to_string(&path)
            .map_err(io(&path))?
            .replace("prettier --write .", "prettier --write src");
        fs::write(&path, text).map_err(io(&path))?;
        let found = changes(&path, HookFormat::Claude, Some(&record));
        assert_eq!(found.added, vec![]);
        assert_eq!(found.edited.len(), 1);
        assert_eq!(found.edited[0].0, "format");
        assert_eq!(found.edited[0].1.command, "prettier --write src");
        Ok(())
    }

    #[test]
    fn a_hook_the_user_already_has_is_not_added_twice() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"hooks":{"PostToolUse":[{"matcher":"Edit|Write","hooks":[{"command":"prettier --write .","type":"command"}]}]}}"#,
        )
        .map_err(io(&path))?;
        let record = write(&path, HookFormat::Claude, &[format_hook()], None)?
            .map_err(SyncError::Invalid)?;
        assert!(record.written.is_empty());
        assert_eq!(read(&path, HookFormat::Claude)["PostToolUse"].len(), 1);
        Ok(())
    }

    #[test]
    fn a_taken_over_entry_is_replaced_when_the_hook_changes() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#,
        )
        .map_err(io(&path))?;
        let found = changes(&path, HookFormat::Claude, None);
        let Some((event, entry)) = found.added[0].entry.clone() else {
            return Err(SyncError::Invalid("no entry".into()));
        };
        let taken = HookFile {
            written: vec![WrittenHook {
                name: "stop-say".into(),
                event,
                entry,
            }],
            seen: read(&path, HookFormat::Claude),
        };
        let changed = Hook {
            name: "stop-say".into(),
            ..Hook {
                command: "say finished".into(),
                ..found.added[0].hook.clone()
            }
        };
        write(&path, HookFormat::Claude, &[changed], Some(&taken))?.map_err(SyncError::Invalid)?;
        let list = &read(&path, HookFormat::Claude)["Stop"];
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["hooks"][0]["command"], "say finished");
        Ok(())
    }

    #[test]
    fn copilot_gets_a_file_of_its_own() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("hooks/skillpouch.json");
        let record = write(&path, HookFormat::Copilot, &[format_hook()], None)?
            .map_err(SyncError::Invalid)?;
        let value: Value = serde_json::from_slice(&fs::read(&path).map_err(io(&path))?)
            .map_err(|e| SyncError::Invalid(e.to_string()))?;
        assert_eq!(value["version"], 1);
        assert_eq!(value["hooks"]["PostToolUse"][0]["timeoutSec"], 30);
        write(&path, HookFormat::Copilot, &[], Some(&record))?.map_err(SyncError::Invalid)?;
        assert!(!path.exists());
        Ok(())
    }

    fn plugin_hook() -> Hook {
        Hook {
            name: "ponytail:posttooluse:1".into(),
            command: "export CLAUDE_PLUGIN_ROOT='/p'; node \"$CLAUDE_PLUGIN_ROOT/fmt.js\"".into(),
            ..format_hook()
        }
    }

    fn commands(path: &Path) -> Vec<String> {
        read(path, HookFormat::Claude)
            .get("PostToolUse")
            .into_iter()
            .flatten()
            .flat_map(|entry| from_native(HookFormat::Claude, "PostToolUse", entry))
            .map(|hook| hook.command)
            .collect()
    }

    fn edit(path: &Path, from: &str, to: &str) -> Result<()> {
        let text = fs::read_to_string(path).map_err(io(path))?;
        fs::write(path, text.replace(from, to)).map_err(io(path))
    }

    #[test]
    fn a_plugins_hook_edited_by_hand_goes_back_to_the_plugins() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        let hooks = [format_hook(), plugin_hook()];
        let first = write(&path, HookFormat::Claude, &hooks, None)?.map_err(SyncError::Invalid)?;
        // Both edited: the user's own hook and the plugin's.
        edit(&path, "prettier --write .", "prettier --check .")?;
        edit(&path, "fmt.js", "lint.js")?;

        write(&path, HookFormat::Claude, &hooks, Some(&first))?.map_err(SyncError::Invalid)?;
        let now = commands(&path);
        assert!(now.contains(&"prettier --check .".to_string()), "{now:?}");
        assert!(
            !now.iter().any(|command| command.contains("lint.js")),
            "{now:?}"
        );
        let plugin = now
            .iter()
            .filter(|command| command.contains("fmt.js"))
            .count();
        assert_eq!(plugin, 1, "{now:?}");
        Ok(())
    }

    #[test]
    fn a_hook_added_while_a_plugins_went_stays() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        let first = write(&path, HookFormat::Claude, &[plugin_hook()], None)?
            .map_err(SyncError::Invalid)?;
        edit(
            &path,
            &plugin_hook().command.replace('"', "\\\""),
            "eslint --fix .",
        )?;
        assert_eq!(commands(&path), ["eslint --fix ."]);

        write(&path, HookFormat::Claude, &[plugin_hook()], Some(&first))?
            .map_err(SyncError::Invalid)?;
        let now = commands(&path);
        assert!(now.contains(&"eslint --fix .".to_string()), "{now:?}");
        assert!(now.contains(&plugin_hook().command), "{now:?}");
        Ok(())
    }

    #[test]
    fn cursor_files_get_a_version() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("hooks.json");
        write(&path, HookFormat::Cursor, &[format_hook()], None)?.map_err(SyncError::Invalid)?;
        let value: Value = serde_json::from_slice(&fs::read(&path).map_err(io(&path))?)
            .map_err(|e| SyncError::Invalid(e.to_string()))?;
        assert_eq!(value["version"], 1);
        assert_eq!(
            value["hooks"]["postToolUse"][0]["command"],
            "prettier --write ."
        );
        Ok(())
    }
}
