//! A whole plugin folder in the pouch, taken apart for the assistants that
//! can't install Claude Code plugins. Each gets the plugin's skills, agents,
//! commands, hooks and MCP servers the way it gets the pouch's own, so one
//! plugin works in every assistant:
//!
//! ```text
//! plugins/ponytail/.claude-plugin/plugin.json   { "hooks": "./hooks/x.json" }
//! plugins/ponytail/skills/lazy/SKILL.md         → a skill
//! plugins/ponytail/agents/reviewer.md           → an agent
//! plugins/ponytail/commands/git/commit.md       → the command `git/commit`
//! plugins/ponytail/hooks/hooks.json, .mcp.json  → hooks, MCP servers
//! ```
//!
//! The folders and files are Claude Code's defaults; paths in `plugin.json`
//! add to them. Hooks and local MCP servers get `CLAUDE_PLUGIN_ROOT`, the
//! plugin's folder in the pouch, since their scripts read it: servers in
//! their environment, hooks from an `export` in front of the command (on
//! Windows, where there's no shell to rely on, written into the command).

use crate::hooks::{self, Hook};
use crate::item::LocalItem;
use crate::mcp::{self, Server};
use crate::plugins::{self, Kind};
use serde_json::{Value, json};
use sp_harness::{HookFormat, McpFormat};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MANIFEST: &str = ".claude-plugin/plugin.json";
/// What a plugin's scripts read to find its folder.
pub const ROOT_VARIABLE: &str = "CLAUDE_PLUGIN_ROOT";

/// One plugin's parts, with absolute paths into its folder.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Unpacked {
    /// The plugin's own name.
    pub plugin: String,
    /// Name → the skill's folder.
    pub skills: BTreeMap<String, PathBuf>,
    /// Name → the agent's Markdown file.
    pub agents: BTreeMap<String, PathBuf>,
    /// Name (`git/commit`) → the command's Markdown file.
    pub commands: BTreeMap<String, PathBuf>,
    /// Named `<plugin>:<event>:<n>`, which no pouch item can be called.
    pub hooks: Vec<Hook>,
    pub servers: BTreeMap<String, Server>,
}

/// The parts of plugin folder `item`, which lives at `root`; `None` for a
/// plugin reference.
pub fn unpack(item: &LocalItem, root: &Path) -> Option<Unpacked> {
    let Kind::Folder { plugin, .. } = plugins::of(item)? else {
        return None;
    };
    let manifest: Value = item
        .files
        .get(MANIFEST)
        .and_then(|file| serde_json::from_slice(&file.bytes).ok())
        .unwrap_or_default();
    let mut unpacked = Unpacked {
        plugin: plugin.clone(),
        ..Unpacked::default()
    };
    let paths = |key: &str, default: &str| {
        let mut found = vec![default.to_string()];
        found.extend(relative_paths(manifest.get(key)));
        found
    };

    for dir in paths("skills", "skills") {
        for path in item.files.keys() {
            let Some(skill) = path.strip_suffix("/SKILL.md") else {
                continue;
            };
            let name = if skill == dir {
                skill.rsplit('/').next()
            } else {
                skill
                    .strip_prefix(&format!("{dir}/"))
                    .filter(|name| !name.contains('/'))
            };
            if let Some(name) = name {
                unpacked
                    .skills
                    .entry(name.to_string())
                    .or_insert_with(|| root.join(skill));
            }
        }
    }
    for (key, nested, list) in [
        ("agents", false, &mut unpacked.agents),
        ("commands", true, &mut unpacked.commands),
    ] {
        for place in paths(key, key) {
            for (name, path) in documents(item, &place, nested) {
                list.entry(name).or_insert_with(|| root.join(path));
            }
        }
    }

    let mut hook_configs: Vec<Value> = Vec::new();
    let from_file = |path: &str| {
        item.files
            .get(path)
            .and_then(|file| serde_json::from_slice::<Value>(&file.bytes).ok())
    };
    hook_configs.extend(from_file("hooks/hooks.json"));
    let mut servers = from_file(".mcp.json").into_iter().collect::<Vec<_>>();
    for value in listed(manifest.get("hooks")) {
        match value {
            Value::String(path) => hook_configs.extend(from_file(clean(path))),
            Value::Object(_) => hook_configs.push(value.clone()),
            _ => {}
        }
    }
    for value in listed(manifest.get("mcpServers")) {
        match value {
            Value::String(path) => servers.extend(from_file(clean(path))),
            Value::Object(_) => servers.push(value.clone()),
            _ => {}
        }
    }

    let mut count = 0;
    for config in &hook_configs {
        let events = config.get("hooks").unwrap_or(config);
        let Some(events) = events.as_object() else {
            continue;
        };
        for (event, entries) in events {
            for entry in entries.as_array().into_iter().flatten() {
                for hook in hooks::from_native(HookFormat::Claude, event, entry) {
                    count += 1;
                    let hook = Hook {
                        name: format!("{plugin}:{}:{count}", event.to_lowercase()),
                        command: with_root(&hook.command, root),
                        ..hook
                    };
                    if !unpacked.hooks.iter().any(|have| have.same_action(&hook)) {
                        unpacked.hooks.push(hook);
                    }
                }
            }
        }
    }
    for config in &servers {
        let entries = config.get("mcpServers").unwrap_or(config);
        for (name, entry) in entries.as_object().into_iter().flatten() {
            let Some(mut server) = mcp::from_native(McpFormat::Claude, name, entry) else {
                continue;
            };
            substitute(&mut server, root);
            let local = server.get("command").is_some();
            if let Some(env) = server.get_mut("env").and_then(Value::as_object_mut)
                && local
            {
                env.entry(ROOT_VARIABLE)
                    .or_insert_with(|| json!(display(root)));
            }
            unpacked.servers.entry(name.clone()).or_insert(server);
        }
    }
    Some(unpacked)
}

/// `"./x"` and `["./x", "./y"]` → `x`, `y`.
fn relative_paths(value: Option<&Value>) -> Vec<String> {
    listed(value)
        .into_iter()
        .filter_map(Value::as_str)
        .map(|path| clean(path).to_string())
        .filter(|path| !path.is_empty() && !path.split('/').any(|part| part == ".."))
        .collect()
}

fn listed(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(value) => vec![value],
        None => Vec::new(),
    }
}

fn clean(path: &str) -> &str {
    path.trim_start_matches("./").trim_end_matches('/')
}

/// The Markdown documents at `place` (a folder, or one `.md` file), by name.
fn documents<'a>(item: &'a LocalItem, place: &str, nested: bool) -> Vec<(String, &'a str)> {
    if place.ends_with(".md") {
        return item
            .files
            .get_key_value(place)
            .map(|(path, _)| {
                let name = path.rsplit('/').next().unwrap_or(path);
                (name.trim_end_matches(".md").to_string(), path.as_str())
            })
            .into_iter()
            .collect();
    }
    item.files
        .keys()
        .filter_map(|path| {
            let name = path
                .strip_prefix(&format!("{place}/"))?
                .strip_suffix(".md")?;
            (nested || !name.contains('/')).then(|| (name.to_string(), path.as_str()))
        })
        .collect()
}

fn display(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

/// `${CLAUDE_PLUGIN_ROOT}` (and `$CLAUDE_PLUGIN_ROOT`) in every string.
fn substitute(value: &mut Value, root: &Path) {
    match value {
        Value::String(text) => *text = replace_root(text, root),
        Value::Array(items) => items.iter_mut().for_each(|item| substitute(item, root)),
        Value::Object(fields) => fields.values_mut().for_each(|item| substitute(item, root)),
        _ => {}
    }
}

fn replace_root(text: &str, root: &Path) -> String {
    let root = display(root);
    text.replace(&format!("${{{ROOT_VARIABLE}}}"), &root)
        .replace(&format!("${ROOT_VARIABLE}"), &root)
}

/// A hook command that finds its plugin wherever SkillPouch keeps it. The
/// command itself is left as it is, so the shell expands the variable in it.
fn with_root(command: &str, root: &Path) -> String {
    if cfg!(unix) {
        let quoted = display(root).replace('\'', r"'\''");
        format!("export {ROOT_VARIABLE}='{quoted}'; {command}")
    } else {
        replace_root(command, root)
    }
}

/// A hook taken from a plugin, by its name.
pub fn is_part(hook_name: &str) -> bool {
    hook_name.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::File;
    use crate::{Result, SyncError};

    fn not_a_folder() -> SyncError {
        SyncError::Invalid("not a plugin folder".into())
    }

    fn plugin(files: &[(&str, &str)]) -> LocalItem {
        LocalItem {
            kind: "plugin".into(),
            name: "ponytail".into(),
            files: files
                .iter()
                .map(|(path, text)| (path.to_string(), File::new(*text)))
                .collect(),
        }
    }

    #[test]
    fn finds_every_part_in_the_default_places() -> Result<()> {
        let item = plugin(&[
            (MANIFEST, r#"{ "name": "ponytail" }"#),
            ("skills/lazy/SKILL.md", "---\nname: lazy\n---\n"),
            ("skills/lazy/ref.md", "more"),
            ("agents/reviewer.md", "---\nname: reviewer\n---\n"),
            ("commands/git/commit.md", "Commit."),
            ("commands/notes.toml", "for another assistant"),
            (
                "hooks/hooks.json",
                r#"{ "hooks": { "PostToolUse": [ { "matcher": "Edit",
                    "hooks": [ { "type": "command", "command": "fmt", "timeout": 5 } ] } ] } }"#,
            ),
            (
                ".mcp.json",
                r#"{ "mcpServers": { "docs": { "type": "http", "url": "https://x/mcp" } } }"#,
            ),
        ]);
        let root = Path::new("/p/ponytail");
        let unpacked = unpack(&item, root).ok_or_else(not_a_folder)?;
        assert_eq!(unpacked.skills["lazy"], root.join("skills/lazy"));
        assert_eq!(unpacked.agents["reviewer"], root.join("agents/reviewer.md"));
        assert_eq!(unpacked.commands.keys().collect::<Vec<_>>(), ["git/commit"]);
        assert_eq!(unpacked.hooks.len(), 1);
        assert_eq!(unpacked.hooks[0].name, "ponytail:posttooluse:1");
        assert_eq!(unpacked.hooks[0].matcher, "Edit");
        assert_eq!(unpacked.hooks[0].timeout, Some(5));
        assert_eq!(unpacked.servers["docs"]["url"], "https://x/mcp");
        Ok(())
    }

    #[test]
    fn manifest_paths_add_to_the_defaults_and_the_root_is_filled_in() -> Result<()> {
        let item = plugin(&[
            (
                MANIFEST,
                r#"{ "name": "ponytail", "hooks": "./config/hooks.json",
                    "agents": ["./extra/auditor.md"], "skills": "./more",
                    "mcpServers": { "db": { "command": "${CLAUDE_PLUGIN_ROOT}/bin/db",
                        "args": ["--data", "${CLAUDE_PLUGIN_ROOT}/data"] } } }"#,
            ),
            ("more/fast/SKILL.md", "---\nname: fast\n---\n"),
            ("extra/auditor.md", "---\nname: auditor\n---\n"),
            (
                "config/hooks.json",
                r#"{ "hooks": { "SessionStart": [ { "hooks": [ { "type": "command",
                    "command": "node \"${CLAUDE_PLUGIN_ROOT}/hooks/start.js\"" } ] } ] } }"#,
            ),
        ]);
        let root = Path::new("/p/ponytail");
        let unpacked = unpack(&item, root).ok_or_else(not_a_folder)?;
        assert_eq!(unpacked.skills["fast"], root.join("more/fast"));
        assert_eq!(unpacked.agents["auditor"], root.join("extra/auditor.md"));
        let command = &unpacked.hooks[0].command;
        let expected = if cfg!(unix) {
            "export CLAUDE_PLUGIN_ROOT='/p/ponytail'; node \"${CLAUDE_PLUGIN_ROOT}/hooks/start.js\""
        } else {
            "node \"/p/ponytail/hooks/start.js\""
        };
        assert_eq!(command, expected);
        let server = &unpacked.servers["db"];
        assert_eq!(server["command"], "/p/ponytail/bin/db");
        assert_eq!(server["args"][1], "/p/ponytail/data");
        assert_eq!(server["env"][ROOT_VARIABLE], "/p/ponytail");
        Ok(())
    }

    /// The root goes in as one quoted word; the command is never rewritten,
    /// so nothing in the path can turn into shell syntax.
    #[cfg(unix)]
    #[test]
    fn an_odd_folder_name_stays_one_word() {
        let command = with_root("a && b", Path::new("/x/it's $(here)"));
        assert_eq!(
            command,
            r#"export CLAUDE_PLUGIN_ROOT='/x/it'\''s $(here)'; a && b"#
        );
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(with_root(
                "printf %s \"$CLAUDE_PLUGIN_ROOT\"",
                Path::new("/x/it's $(here)"),
            ))
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        assert_eq!(output, "/x/it's $(here)");
    }

    #[test]
    fn a_plugin_reference_has_nothing_to_unpack() {
        let item = LocalItem {
            kind: "plugin".into(),
            name: "wakatime".into(),
            files: [(
                "plugin.json".to_string(),
                File::new(r#"{ "name": "wakatime", "npm": "opencode-wakatime" }"#),
            )]
            .into(),
        };
        assert_eq!(unpack(&item, Path::new("/p")), None);
    }

    #[test]
    fn paths_out_of_the_plugin_are_ignored() -> Result<()> {
        let item = plugin(&[
            (MANIFEST, r#"{ "name": "p", "agents": "../../elsewhere" }"#),
            ("agents/a.md", "a"),
        ]);
        let unpacked = unpack(&item, Path::new("/p")).ok_or_else(not_a_folder)?;
        assert_eq!(unpacked.agents.keys().collect::<Vec<_>>(), ["a"]);
        Ok(())
    }
}
