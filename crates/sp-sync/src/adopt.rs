//! Bringing what an assistant already has into the pouch folder, in the
//! pouch's neutral format: `CLAUDE.md` → `instructions/instructions/
//! INSTRUCTIONS.md`, `agents/x.md` → `agents/x/AGENT.md`, an MCP entry in
//! a config → `mcp-servers/x/server.json`.

use crate::item::{self, File, Files, LocalItem, PouchDir, Skipped};
use crate::{Result, mcp};
use sp_harness::{Harness, Kind, McpFormat};
use std::fs;
use std::path::PathBuf;

/// An item found in an assistant, ready for the pouch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub item: LocalItem,
    /// The file or folder it came from; moved to the backups once the
    /// pouch's link replaces it. `None` for MCP servers (config entries).
    pub origin: Option<PathBuf>,
}

/// How an assistant writes MCP servers in its config.
pub fn format_of(harness_id: &str) -> McpFormat {
    match harness_id {
        "opencode" => McpFormat::OpenCode,
        "codex" => McpFormat::Codex,
        "gemini-cli" | "qwen-code" => McpFormat::Gemini,
        "copilot" => McpFormat::Copilot,
        "windsurf" => McpFormat::Windsurf,
        _ => McpFormat::Claude,
    }
}

/// What `harness` holds, as pouch items; unnamed items are left out. In a
/// git checkout the originals stay where they are (no `origin`).
pub fn found_in(harness: &Harness) -> Vec<Found> {
    let movable = sp_harness::git_checkout(&harness.root).is_none();
    let mut out = Vec::new();
    for found in &harness.items {
        let single = |main: &str| -> Option<Files> {
            let bytes = fs::read(&found.path).ok()?;
            let mut files = Files::new();
            files.insert(main.to_string(), File::new(bytes));
            Some(files)
        };
        let (kind, name, files, origin) = match found.kind {
            Kind::Instructions => (
                "instructions",
                Some(item::INSTRUCTIONS.to_string()),
                single("INSTRUCTIONS.md"),
                true,
            ),
            Kind::Skill => {
                let mut skipped = Skipped::default();
                let files = item::read_files(&found.path, &found.name, &mut skipped).ok();
                let folder = found
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // The link goes where the folder was, so the folder name wins.
                ("skill", item::safe_name(&folder), files, true)
            }
            Kind::Agent => (
                "agent",
                item::safe_name(&found.name),
                single("AGENT.md"),
                true,
            ),
            Kind::Command if found.path.extension().is_some_and(|ext| ext == "md") => (
                "command",
                item::safe_name(&found.name),
                single("COMMAND.md"),
                true,
            ),
            Kind::Command => {
                let markdown = fs::read_to_string(&found.path)
                    .ok()
                    .and_then(|text| crate::command::from_toml(&text, ""));
                let files = markdown.map(|text| {
                    let mut files = Files::new();
                    files.insert("COMMAND.md".to_string(), File::new(text));
                    files
                });
                ("command", item::safe_name(&found.name), files, true)
            }
            Kind::McpServer => {
                let servers = mcp::read(&found.path, format_of(harness.id));
                let files = servers.get(&found.name).map(|server| {
                    let mut text = serde_json::to_string_pretty(server).unwrap_or_default();
                    text.push('\n');
                    let mut files = Files::new();
                    files.insert("server.json".to_string(), File::new(text));
                    files
                });
                ("mcp_server", item::safe_name(&found.name), files, false)
            }
        };
        let (Some(name), Some(files)) = (name, files) else {
            continue;
        };
        if files.is_empty() {
            continue;
        }
        out.push(Found {
            item: LocalItem {
                kind: kind.to_string(),
                name,
                files,
            },
            origin: (origin && movable).then(|| found.path.clone()),
        });
    }
    out
}

/// What putting found items into the folder did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Adopted {
    pub added: usize,
    /// Same name, different content: kept as `<name>-conflict`.
    pub copies: usize,
    /// Where originals were moved.
    pub backed_up: Vec<PathBuf>,
}

/// Puts `found` into the pouch folder. An item the folder already has with
/// other content is kept as a `-conflict` copy, unless `replace`. With
/// `backups`, each original is moved there afterwards (its link replaces it).
pub fn adopt(
    dir: &PouchDir,
    found: &[Found],
    replace: bool,
    backups: Option<&std::path::Path>,
) -> Result<Adopted> {
    let (mut existing, _) = dir.read()?;
    let mut adopted = Adopted::default();
    for Found { item, origin } in found {
        let key = item::key(&item.kind, &item.name);
        // Already in the pouch, maybe under another name (a conflict copy).
        let known = !replace
            && existing
                .values()
                .any(|other| other.kind == item.kind && other.files == item.files);
        let name = match existing.get(&key) {
            Some(current) if current.files == item.files => None,
            _ if known => None,
            Some(_) if !replace => {
                let taken = |name: &str| existing.contains_key(&item::key(&item.kind, name));
                adopted.copies += 1;
                Some(crate::engine::conflict_name(&item.name, &taken))
            }
            _ => Some(item.name.clone()),
        };
        if let Some(name) = name {
            dir.write_item(&item.kind, &name, &item.files)?;
            if name == item.name {
                adopted.added += 1;
            }
            existing.insert(
                item::key(&item.kind, &name),
                LocalItem {
                    name,
                    ..item.clone()
                },
            );
        }
        if let (Some(origin), Some(backups)) = (origin, backups)
            && fs::symlink_metadata(origin).is_ok()
        {
            let aside = crate::project::back_up(origin, backups)?;
            adopted.backed_up.push(aside);
        }
    }
    Ok(adopted)
}

/// Moves every original of `found` into the backups, without adopting them.
pub fn set_aside(found: &[Found], backups: &std::path::Path) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::new();
    for origin in found.iter().filter_map(|found| found.origin.as_ref()) {
        if fs::symlink_metadata(origin).is_ok() {
            moved.push(crate::project::back_up(origin, backups)?);
        }
    }
    Ok(moved)
}

/// Writes a test file, making its folder.
#[cfg(test)]
use crate::error::io;

#[cfg(test)]
fn touch(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    fs::write(path, text).map_err(io(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_harness::Places;
    use std::path::Path;

    fn places(home: &Path) -> Places {
        Places::plain(home, &home.join(".skillpouch"))
    }

    #[test]
    fn turns_an_assistant_into_pouch_items() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let h = tmp.path();
        touch(&h.join(".claude/CLAUDE.md"), "# Rules")?;
        touch(
            &h.join(".claude/skills/pdf/SKILL.md"),
            "---\nname: PDF Tools\n---",
        )?;
        touch(&h.join(".claude/agents/reviewer.md"), "review")?;
        touch(&h.join(".claude/commands/git/commit.md"), "commit")?;
        touch(
            &h.join(".claude.json"),
            r#"{"mcpServers":{"gh":{"command":"gh"}}}"#,
        )?;
        let harness = sp_harness::discover(&places(h)).remove(0);
        let found = found_in(&harness);
        let names: Vec<String> = found
            .iter()
            .map(|f| item::key(&f.item.kind, &f.item.name))
            .collect();
        assert_eq!(
            names,
            [
                "instructions/instructions",
                "skill/pdf",
                "agent/reviewer",
                "command/git-commit",
                "mcp_server/gh"
            ]
        );
        assert!(found[3].item.files.contains_key("COMMAND.md"));
        assert_eq!(found[4].origin, None);

        let pouch = PouchDir::new(h.join("pouch"));
        let backups = h.join("backups");
        let adopted = adopt(&pouch, &found, false, Some(&backups))?;
        assert_eq!(adopted.added, 5);
        assert_eq!(adopted.backed_up.len(), 4);
        assert!(!h.join(".claude/CLAUDE.md").exists());
        assert_eq!(pouch.read()?.0.len(), 5);

        // The same name with other content from a second assistant is a copy.
        let mut other = found[0].clone();
        other
            .item
            .files
            .insert("INSTRUCTIONS.md".into(), File::new("other rules"));
        other.origin = None;
        let adopted = adopt(&pouch, &[other.clone()], false, None)?;
        assert_eq!((adopted.added, adopted.copies), (0, 1));
        assert!(
            pouch
                .read()?
                .0
                .contains_key("instructions/instructions-conflict")
        );
        // ...unless this computer's version replaces it.
        adopt(&pouch, &[other], true, None)?;
        let (items, _) = pouch.read()?;
        assert_eq!(
            items["instructions/instructions"].files["INSTRUCTIONS.md"].bytes,
            b"other rules"
        );
        Ok(())
    }

    #[test]
    fn gemini_commands_come_in_as_markdown() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let h = tmp.path();
        touch(
            &h.join(".gemini/commands/review.toml"),
            "description = \"Review code\"\nprompt = \"Review {{args}}\"\n",
        )?;
        let harness = sp_harness::discover(&places(h)).remove(0);
        let found = found_in(&harness);
        assert_eq!(found.len(), 1);
        let text = String::from_utf8_lossy(&found[0].item.files["COMMAND.md"].bytes).into_owned();
        assert_eq!(
            text,
            "---\ndescription: Review code\n---\n\nReview $ARGUMENTS"
        );
        Ok(())
    }

    #[test]
    fn files_in_a_git_checkout_stay_put() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let h = tmp.path();
        touch(&h.join(".claude/.git/HEAD"), "ref")?;
        touch(&h.join(".claude/CLAUDE.md"), "# Rules")?;
        let harness = sp_harness::discover(&places(h)).remove(0);
        let found = found_in(&harness);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].origin, None);
        let pouch = PouchDir::new(h.join("pouch"));
        let adopted = adopt(&pouch, &found, false, Some(&h.join("backups")))?;
        assert!(adopted.backed_up.is_empty());
        assert!(h.join(".claude/CLAUDE.md").exists());
        Ok(())
    }

    #[test]
    fn content_the_pouch_has_is_not_adopted_again() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let pouch = PouchDir::new(tmp.path().join("pouch"));
        let rules = |text: &str| Found {
            item: LocalItem {
                kind: "instructions".into(),
                name: item::INSTRUCTIONS.into(),
                files: [("INSTRUCTIONS.md".to_string(), File::new(text))].into(),
            },
            origin: None,
        };
        adopt(&pouch, &[rules("mine")], false, None)?;
        adopt(&pouch, &[rules("theirs")], false, None)?;
        // Found again on the next sync: nothing new.
        let again = adopt(&pouch, &[rules("theirs"), rules("mine")], false, None)?;
        assert_eq!((again.added, again.copies), (0, 0));
        assert_eq!(pouch.read()?.0.len(), 2);
        Ok(())
    }
}
