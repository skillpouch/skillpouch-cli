//! What the user did to the pouch's items inside an assistant since the last
//! sync, for example with the assistant's own commands (`claude mcp remove`,
//! `claude plugin uninstall`, deleting a skill folder):
//!
//!  - a link or copy of ours that is gone, an MCP server, hook or plugin of
//!    ours taken out of a config: the item is **removed** from the pouch;
//!  - a link of ours replaced by a real file or folder (a reinstall, an
//!    editor that saves by renaming): its content **replaces** the item.
//!
//! Only places whose folder or config file is still there count: an
//! assistant that was uninstalled, or a config that no longer parses, says
//! nothing about the items. Instructions are shared by every assistant, so a
//! deleted `CLAUDE.md` is linked again rather than removing them everywhere.

use crate::Result;
use crate::hooks;
use crate::item::{self, File, Files, LocalItem, PouchDir, Skipped};
use crate::mcp;
use crate::plugins::{self, Kind as PluginKind};
use crate::project::{Projection, current};
use sp_core::Home;
use sp_harness::{McpFormat, PluginFormat, Target};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Links of ours that may vanish at once (from one folder, or half of all
/// of them) before it looks like a wipe rather than items deleted.
const WIPE_AT: usize = 3;

/// An item to take out of the pouch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub kind: String,
    pub name: String,
    /// Where it was removed: the link, or the config file.
    pub at: PathBuf,
}

/// An item whose content was replaced in an assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    pub kind: String,
    pub name: String,
    pub files: Files,
    pub at: PathBuf,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Noticed {
    pub removed: Vec<Removed>,
    pub replaced: Vec<Replaced>,
    /// Removals not acted on, with why.
    pub held_back: Vec<String>,
}

pub fn noticed(
    home: &Home,
    dir: &PouchDir,
    targets: &[Target],
    previous: &Projection,
) -> Result<Noticed> {
    let (items, _) = dir.read()?;
    let base = current(home);
    let mut found = Noticed::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut remove = |found: &mut Noticed, kind: &str, name: &str, at: &Path| {
        if items.contains_key(&item::key(kind, name)) && seen.insert(item::key(kind, name)) {
            found.removed.push(Removed {
                kind: kind.into(),
                name: name.into(),
                at: at.to_path_buf(),
            });
        }
    };
    let writable: Vec<&Target> = targets.iter().filter(|t| t.read_only.is_none()).collect();
    let inside_writable =
        |path: &Path| writable.iter().any(|target| path.starts_with(&target.root));

    // Links: gone, or something else in their place.
    let mut gone: BTreeMap<PathBuf, Vec<(PathBuf, String, String)>> = BTreeMap::new();
    let mut ours_per_folder: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for (path, to) in &previous.links {
        let Some((kind, name)) = item_of(&base, to) else {
            continue;
        };
        let Some(folder) = path.parent() else {
            continue;
        };
        if !inside_writable(path) {
            continue;
        }
        *ours_per_folder.entry(folder.to_path_buf()).or_default() += 1;
        if fs::read_link(path).is_ok_and(|now| &now == to) {
            continue;
        }
        if fs::symlink_metadata(path).is_err() {
            if folder.is_dir() && kind != "instructions" {
                gone.entry(folder.to_path_buf())
                    .or_default()
                    .push((path.clone(), kind, name));
            }
            continue;
        }
        let Some(current) = items.get(&item::key(&kind, &name)) else {
            continue;
        };
        if let Some(files) = content_at(path, current)
            && files != current.files
        {
            found.replaced.push(Replaced {
                kind,
                name,
                files,
                at: path.clone(),
            });
        }
    }
    for (folder, links) in gone {
        let all = ours_per_folder.get(&folder).copied().unwrap_or(0);
        if links.len() >= WIPE_AT && links.len() == all {
            found.held_back.push(format!(
                "every SkillPouch item in {} disappeared at once, so they were linked again \
                 instead of deleted; delete them one by one, or in the pouch",
                folder.display()
            ));
            continue;
        }
        for (path, kind, name) in links {
            remove(&mut found, &kind, &name, &path);
        }
    }

    // Copies (Gemini's TOML commands, Windows without links).
    for (path, copied) in &previous.copies {
        let Some((kind, name)) = item_of(&base, &copied.from) else {
            continue;
        };
        let folder_there = path.parent().is_some_and(Path::is_dir);
        if kind != "instructions"
            && folder_there
            && inside_writable(path)
            && fs::symlink_metadata(path).is_err()
        {
            remove(&mut found, &kind, &name, path);
        }
    }

    for target in &writable {
        if let Some((path, format)) = &target.mcp
            && let Some(written) = previous.mcp.get(path)
            && parses(path, *format == McpFormat::Codex)
        {
            let now = mcp::read(path, *format);
            for name in written.keys().filter(|name| !now.contains_key(*name)) {
                remove(&mut found, "mcp_server", name, path);
            }
        }
        if let Some((path, format)) = &target.hooks
            && let Some(written) = previous.hooks.get(path)
            && parses(path, false)
        {
            let now = hooks::read(path, *format);
            let edited: BTreeSet<String> = hooks::changes(path, *format, Some(written))
                .edited
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            let mut names: BTreeMap<&str, bool> = BTreeMap::new();
            for entry in &written.written {
                let present = now
                    .get(&entry.event)
                    .is_some_and(|list| list.contains(&entry.entry));
                *names.entry(&entry.name).or_default() |= present;
            }
            for (name, present) in names {
                if !present && !edited.contains(name) {
                    remove(&mut found, "hook", name, path);
                }
            }
        }
        if let Some((path, format)) = &target.plugins
            && let Some(written) = previous.plugins.get(path)
            && parses(path, false)
        {
            let on = plugins::turned_on(path, *format);
            let ours = match format {
                PluginFormat::Claude | PluginFormat::Factory => &written.enabled,
                PluginFormat::OpenCode => &written.npm,
            };
            for id in ours.iter().filter(|id| !on.contains(*id)) {
                if let Some(item) = plugin_item(&items, id) {
                    remove(&mut found, "plugin", &item.name, path);
                }
            }
        }
    }
    // Many of ours gone in one go looks like a cleanup, a reinstall or
    // another tool at work, not like someone deleting things one by one.
    let ours = previous.links.len() + previous.copies.len();
    let is_file = |removed: &Removed| {
        previous.links.contains_key(&removed.at) || previous.copies.contains_key(&removed.at)
    };
    let files = found
        .removed
        .iter()
        .filter(|removed| is_file(removed))
        .count();
    if files >= WIPE_AT && files * 2 >= ours {
        found.held_back.push(format!(
            "{files} SkillPouch items disappeared from your assistants at once, so they were \
             linked again instead of deleted; delete them one by one, or in the pouch"
        ));
        found.removed.retain(|removed| !is_file(removed));
    }
    Ok(found)
}

/// The pouch item a link or copy points into: `<base>/<folder>/<name>/…`.
/// A plugin's part (`<base>/plugins/<name>/skills/x`) isn't an item: losing
/// it in one assistant doesn't take the whole plugin out of the pouch.
fn item_of(base: &Path, to: &Path) -> Option<(String, String)> {
    let mut parts = to.strip_prefix(base).ok()?.components();
    let folder = parts.next()?.as_os_str().to_str()?;
    let name = parts.next()?.as_os_str().to_str()?;
    let (kind, _, main) = item::KINDS.iter().find(|kind| kind.1 == folder)?;
    match parts.next() {
        Some(file) if file.as_os_str() != *main || parts.next().is_some() => None,
        _ => Some((kind.to_string(), name.to_string())),
    }
}

/// What is at `path` now, as the item's files: a whole folder for a skill,
/// else the main file (the item's other files kept).
fn content_at(path: &Path, current: &LocalItem) -> Option<Files> {
    if current.kind == "skill" {
        let mut skipped = Skipped::default();
        let files = item::read_files(path, &current.name, &mut skipped).ok()?;
        return (!files.is_empty() && fs::metadata(path).ok()?.is_dir()).then_some(files);
    }
    let main = item::main_file(&current.kind)?;
    let bytes = fs::read(path).ok()?;
    let mut files = current.files.clone();
    let exec = files.get(main).is_some_and(|file| file.exec);
    files.insert(main.to_string(), File { bytes, exec });
    Some(files)
}

/// The file is there and still parses, so what's missing from it was removed.
fn parses(path: &Path, toml: bool) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    if toml {
        return text.parse::<toml::Table>().is_ok();
    }
    text.trim().is_empty()
        || serde_json::from_str::<serde_json::Value>(&sp_harness::strip_jsonc(&text)).is_ok()
}

/// The plugin item behind a turned-on id (`plugin@market`) or npm package.
fn plugin_item<'a>(items: &'a BTreeMap<String, LocalItem>, id: &str) -> Option<&'a LocalItem> {
    items
        .values()
        .filter(|item| item.kind == "plugin")
        .find(|item| match plugins::of(item) {
            Some(PluginKind::Folder { plugin, .. }) => {
                id == format!("{plugin}@{}", plugins::MARKETPLACE)
            }
            Some(PluginKind::Reference(plugin)) => {
                plugin.source.as_deref() == Some(id) || plugin.npm.as_deref() == Some(id)
            }
            None => false,
        })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::error::io;
    use crate::project::{project, set_current};
    use sp_harness::CommandFormat;

    #[test]
    fn a_plugins_parts_are_not_items() {
        let base = Path::new("/sp/current");
        let of = |to: &str| item_of(base, &base.join(to));
        assert_eq!(of("skills/pdf"), Some(("skill".into(), "pdf".into())));
        assert_eq!(
            of("agents/reviewer/AGENT.md"),
            Some(("agent".into(), "reviewer".into()))
        );
        assert_eq!(of("plugins/ponytail/skills/terse"), None);
        assert_eq!(of("plugins/ponytail/agents/reviewer.md"), None);
        assert_eq!(of("skills/pdf/notes/ref.md"), None);
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        home: Home,
        dir: PouchDir,
        claude: PathBuf,
        json: PathBuf,
        targets: Vec<Target>,
        projection: Projection,
    }

    fn setup(skills: &[&str]) -> Result<Setup> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = PouchDir::new(&pouch);
        let claude = tmp.path().join(".claude");
        let json = tmp.path().join(".claude.json");
        fs::create_dir_all(&claude).map_err(io(&claude))?;
        fs::write(&json, "{}").map_err(io(&json))?;
        let one =
            |text: &str, main: &str| -> Files { [(main.to_string(), File::new(text))].into() };
        dir.write_item(
            "instructions",
            "instructions",
            &one("# Rules", "INSTRUCTIONS.md"),
        )?;
        for skill in skills {
            dir.write_item("skill", skill, &one("# Skill", "SKILL.md"))?;
        }
        dir.write_item("agent", "reviewer", &one("review", "AGENT.md"))?;
        dir.write_item(
            "mcp_server",
            "gh",
            &one(
                r#"{"name":"gh","command":"npx","args":[],"env":{}}"#,
                "server.json",
            ),
        )?;
        set_current(&home, &pouch)?;
        let targets = vec![Target {
            id: "claude-code",
            name: "Claude Code",
            root: claude.clone(),
            settings: claude.clone(),
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
        let (projection, _) = project(&home, &items, &targets, &backups, &Projection::default())?;
        Ok(Setup {
            _tmp: tmp,
            home,
            dir,
            claude,
            json,
            targets,
            projection,
        })
    }

    fn check(s: &Setup) -> Result<Noticed> {
        noticed(&s.home, &s.dir, &s.targets, &s.projection)
    }

    fn removed(found: &Noticed) -> Vec<String> {
        found
            .removed
            .iter()
            .map(|r| format!("{}/{}", r.kind, r.name))
            .collect()
    }

    #[test]
    fn nothing_changed_nothing_noticed() -> Result<()> {
        let s = setup(&["pdf"])?;
        assert_eq!(check(&s)?, Noticed::default());
        Ok(())
    }

    #[test]
    fn deleting_in_an_assistant_removes_from_the_pouch() -> Result<()> {
        let s = setup(&["pdf", "docx"])?;
        let link = s.claude.join("skills/pdf");
        fs::remove_file(&link).map_err(io(&link))?;
        let agent = s.claude.join("agents/reviewer.md");
        fs::remove_file(&agent).map_err(io(&agent))?;
        // `claude mcp remove gh`
        fs::write(&s.json, r#"{"mcpServers":{}}"#).map_err(io(&s.json))?;
        let found = check(&s)?;
        assert_eq!(
            removed(&found),
            ["agent/reviewer", "skill/pdf", "mcp_server/gh"]
        );
        assert!(found.replaced.is_empty());
        Ok(())
    }

    #[test]
    fn instructions_and_uninstalled_assistants_are_left_alone() -> Result<()> {
        let s = setup(&["pdf"])?;
        let rules = s.claude.join("CLAUDE.md");
        fs::remove_file(&rules).map_err(io(&rules))?;
        assert!(removed(&check(&s)?).is_empty());
        // Uninstalled: the whole folder and its config are gone.
        fs::remove_dir_all(&s.claude).map_err(io(&s.claude))?;
        fs::remove_file(&s.json).map_err(io(&s.json))?;
        assert!(removed(&check(&s)?).is_empty());
        Ok(())
    }

    #[test]
    fn a_broken_config_says_nothing_about_its_servers() -> Result<()> {
        let s = setup(&[])?;
        fs::write(&s.json, "{ half").map_err(io(&s.json))?;
        assert!(removed(&check(&s)?).is_empty());
        Ok(())
    }

    #[test]
    fn a_wiped_folder_is_not_a_mass_delete() -> Result<()> {
        let s = setup(&["a", "b", "c"])?;
        let skills = s.claude.join("skills");
        fs::remove_dir_all(&skills).map_err(io(&skills))?;
        fs::create_dir_all(&skills).map_err(io(&skills))?;
        let found = check(&s)?;
        assert!(removed(&found).is_empty());
        assert_eq!(found.held_back.len(), 1);
        Ok(())
    }

    #[test]
    fn many_disappearing_at_once_is_not_a_mass_delete() -> Result<()> {
        let s = setup(&["a", "b"])?;
        for path in ["skills/a", "skills/b", "agents/reviewer.md"] {
            let link = s.claude.join(path);
            fs::remove_file(&link).map_err(io(&link))?;
        }
        let found = check(&s)?;
        assert!(removed(&found).is_empty(), "{found:?}");
        assert_eq!(found.held_back.len(), 1);
        Ok(())
    }

    #[test]
    fn a_reinstalled_skill_replaces_the_pouch_copy() -> Result<()> {
        let s = setup(&["pdf"])?;
        let link = s.claude.join("skills/pdf");
        fs::remove_file(&link).map_err(io(&link))?;
        fs::create_dir_all(link.join("scripts")).map_err(io(&link))?;
        fs::write(link.join("SKILL.md"), "# PDF v2").map_err(io(&link))?;
        fs::write(link.join("scripts/fill.py"), "print(2)").map_err(io(&link))?;
        // An editor that saves by rename turns the agent link into a file.
        let agent = s.claude.join("agents/reviewer.md");
        fs::remove_file(&agent).map_err(io(&agent))?;
        fs::write(&agent, "review harder").map_err(io(&agent))?;
        let found = check(&s)?;
        assert!(found.removed.is_empty());
        let replaced: BTreeMap<String, &Files> = found
            .replaced
            .iter()
            .map(|r| (format!("{}/{}", r.kind, r.name), &r.files))
            .collect();
        assert_eq!(replaced["skill/pdf"].len(), 2);
        assert_eq!(
            replaced["agent/reviewer"]["AGENT.md"].bytes,
            b"review harder".to_vec()
        );
        Ok(())
    }
}
