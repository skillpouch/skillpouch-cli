//! Handing items back to an assistant: SkillPouch stops managing them, and
//! the assistant keeps working as if SkillPouch had never been there. Links
//! become real files and folders; copies, MCP servers, hooks and plugins
//! stay where they are and are simply forgotten.
//!
//! Used for `skillpouch unlink` and when this computer turns an assistant
//! or a kind off.

use crate::error::io;
use crate::item;
use crate::project::{Projection, current};
use crate::{Result, plugins};
use sp_core::Home;
use sp_harness::Target;
use std::fs;
use std::path::{Path, PathBuf};

/// What `release` handed back.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Released {
    /// Links replaced by real files or folders.
    pub files: usize,
    /// Config entries (MCP servers, hooks, plugins) no longer managed.
    pub entries: usize,
}

/// Releases everything in `projection` that `wanted(assistant, kind)` says
/// so for. `assistant` is the id of the assistant a place belongs to, if
/// one is known.
pub fn release(
    home: &Home,
    projection: &mut Projection,
    targets: &[Target],
    wanted: &dyn Fn(Option<&str>, &str) -> bool,
) -> Result<Released> {
    let base = current(home);
    let mut released = Released::default();
    let config_owner = |path: &Path| -> Option<&str> {
        targets
            .iter()
            .find(|target| {
                target.mcp.as_ref().is_some_and(|(p, _)| p == path)
                    || target.hooks.as_ref().is_some_and(|(p, _)| p == path)
                    || target.plugins.as_ref().is_some_and(|(p, _)| p == path)
            })
            .map(|target| target.id)
    };
    let owner = |path: &Path| -> Option<&str> {
        targets
            .iter()
            .find(|target| path.starts_with(&target.root))
            .map(|target| target.id)
            .or_else(|| config_owner(path))
    };

    let links: Vec<(PathBuf, PathBuf)> = projection
        .links
        .iter()
        .map(|(path, to)| (path.clone(), to.clone()))
        .collect();
    for (path, to) in links {
        let Some(kind) = kind_of(&base, &to) else {
            continue;
        };
        if !wanted(owner(&path), &kind) {
            continue;
        }
        if fs::read_link(&path).is_ok_and(|now| now == to) {
            materialize(&path, &to)?;
            released.files += 1;
        }
        projection.links.remove(&path);
    }
    let copies: Vec<PathBuf> = projection
        .copies
        .iter()
        .filter(|(path, copied)| {
            kind_of(&base, &copied.from).is_some_and(|kind| wanted(owner(path), &kind))
        })
        .map(|(path, _)| path.clone())
        .collect();
    for path in copies {
        projection.copies.remove(&path);
        released.files += 1;
    }
    for (path, servers) in &mut projection.mcp {
        if wanted(config_owner(path), "mcp_server") {
            released.entries += servers.len();
            servers.clear();
        }
    }
    projection.mcp.retain(|_, servers| !servers.is_empty());
    for (path, file) in &mut projection.hooks {
        if wanted(config_owner(path), "hook") {
            released.entries += file.written.len();
            file.written.clear();
        }
    }
    for (path, file) in &mut projection.plugins {
        if wanted(config_owner(path), "plugin") {
            released.entries += file.enabled.len() + file.npm.len();
            *file = plugins::PluginFile {
                seen: file.seen.clone(),
                ..plugins::PluginFile::default()
            };
        }
    }
    Ok(released)
}

/// Before this computer starts over with the pouch folder `next` (a new
/// login, another account or server): everything still linked into the
/// pouch folder used so far becomes the assistants' own again, so setting up
/// finds it as real content instead of links that are about to point
/// elsewhere. `None` when `next` is already the folder in use.
pub fn leave_pouch(
    home: &Home,
    projection: &mut Projection,
    targets: &[Target],
    next: &Path,
) -> Result<Option<Released>> {
    let Ok(used) = fs::read_link(current(home)) else {
        return Ok(None);
    };
    if used == next || !used.exists() {
        return Ok(None);
    }
    release(home, projection, targets, &|_, _| true).map(Some)
}

/// The kind of the pouch item a link or copy points into.
fn kind_of(base: &Path, to: &Path) -> Option<String> {
    let folder = to.strip_prefix(base).ok()?.components().next()?;
    let folder = folder.as_os_str().to_str()?;
    item::KINDS
        .iter()
        .find(|kind| kind.1 == folder)
        .map(|kind| kind.0.to_string())
}

/// Replaces the link at `path` with a real copy of what it points to.
fn materialize(path: &Path, to: &Path) -> Result<()> {
    let staged = path.with_file_name(format!(
        ".{}.sp-unlink",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let _ = fs::remove_dir_all(&staged);
    let _ = fs::remove_file(&staged);
    if to.is_dir() {
        copy_tree(to, &staged)?;
    } else {
        fs::copy(to, &staged).map_err(io(to))?;
    }
    fs::remove_file(path)
        .or_else(|_| fs::remove_dir(path))
        .map_err(io(path))?;
    fs::rename(&staged, path).map_err(io(path))
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).map_err(io(to))?;
    for entry in fs::read_dir(from).map_err(io(from))?.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            fs::copy(&source, &target).map_err(io(&source))?;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::item::{File, Files, PouchDir};
    use crate::project::{project, set_current};
    use sp_harness::{CommandFormat, McpFormat};

    #[test]
    fn released_items_become_the_assistants_own() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = PouchDir::new(&pouch);
        let claude = tmp.path().join(".claude");
        let json = tmp.path().join(".claude.json");
        fs::create_dir_all(&claude).map_err(io(&claude))?;
        let one =
            |main: &str, text: &str| -> Files { [(main.to_string(), File::new(text))].into() };
        dir.write_item("skill", "pdf", &one("SKILL.md", "# PDF"))?;
        dir.write_item("agent", "reviewer", &one("AGENT.md", "review"))?;
        dir.write_item(
            "mcp_server",
            "gh",
            &one(
                "server.json",
                r#"{"name":"gh","command":"npx","args":[],"env":{}}"#,
            ),
        )?;
        set_current(&home, &pouch)?;
        let targets = vec![Target {
            id: "claude-code",
            name: "Claude Code",
            root: claude.clone(),
            instructions: Some(claude.join("CLAUDE.md")),
            skills: Some(claude.join("skills")),
            agents: Some((claude.join("agents"), sp_harness::AgentFormat::Claude)),
            commands: Some((claude.join("commands"), CommandFormat::Markdown)),
            mcp: Some((json.clone(), McpFormat::Claude)),
            hooks: None,
            plugins: None,
            read_only: None,
        }];
        let (items, _) = dir.read()?;
        let backups = home.root().join("backups/t");
        let (mut projection, _) =
            project(&home, &items, &targets, &backups, &Projection::default())?;

        // Only agents first.
        let only_agents = |_: Option<&str>, kind: &str| kind == "agent";
        let released = release(&home, &mut projection, &targets, &only_agents)?;
        assert_eq!(released.files, 1);
        let agent = claude.join("agents/reviewer.md");
        assert!(fs::read_link(&agent).is_err());
        assert_eq!(fs::read_to_string(&agent).map_err(io(&agent))?, "review");
        assert!(fs::read_link(claude.join("skills/pdf")).is_ok());

        // Then all of Claude Code.
        let claude_only = |owner: Option<&str>, _: &str| owner == Some("claude-code");
        let released = release(&home, &mut projection, &targets, &claude_only)?;
        assert_eq!((released.files, released.entries), (1, 1));
        let skill = claude.join("skills/pdf");
        assert!(fs::read_link(&skill).is_err());
        assert_eq!(
            fs::read_to_string(skill.join("SKILL.md")).map_err(io(&skill))?,
            "# PDF"
        );
        assert!(projection.links.is_empty() && projection.mcp.is_empty());
        // The server entry stays in the config, now the user's.
        let config = fs::read_to_string(&json).map_err(io(&json))?;
        assert!(config.contains("\"gh\""));
        Ok(())
    }

    #[test]
    fn a_new_pouch_finds_what_the_old_one_had_linked_as_real_content() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let old = home.root().join("pouches/old");
        let next = home.root().join("pouches/new");
        let dir = PouchDir::new(&old);
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).map_err(io(&claude))?;
        dir.write_item(
            "skill",
            "pdf",
            &[("SKILL.md".to_string(), File::new("# PDF"))].into(),
        )?;
        dir.write_item(
            "instructions",
            item::INSTRUCTIONS,
            &[("INSTRUCTIONS.md".to_string(), File::new("# Rules"))].into(),
        )?;
        set_current(&home, &old)?;
        let places = sp_harness::Places::plain(tmp.path(), home.root());
        let targets: Vec<Target> = sp_harness::targets(&places)
            .into_iter()
            .filter(|target| target.id == "claude-code")
            .collect();
        let (items, _) = dir.read()?;
        let backups = home.root().join("backups/t");
        let (mut projection, _) =
            project(&home, &items, &targets, &backups, &Projection::default())?;
        // Linked: the assistant has nothing of its own.
        let found = |places: &sp_harness::Places| {
            sp_harness::discover(places)
                .into_iter()
                .find(|harness| harness.id == "claude-code")
                .map_or(0, |harness| harness.items.len())
        };
        assert_eq!(found(&places), 0);

        // Same folder: nothing to do.
        assert_eq!(leave_pouch(&home, &mut projection, &targets, &old)?, None);
        let left = leave_pouch(&home, &mut projection, &targets, &next)?;
        assert_eq!(left.map(|released| released.files), Some(2));
        assert!(projection.links.is_empty());
        assert_eq!(
            fs::read_to_string(claude.join("skills/pdf/SKILL.md")).map_err(io(&claude))?,
            "# PDF"
        );
        assert_eq!(found(&places), 2);
        // The old pouch folder is untouched.
        assert_eq!(dir.read()?.0.len(), 2);
        Ok(())
    }
}
