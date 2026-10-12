//! Making the pouch what every assistant on this computer uses: links from
//! their folders into `~/.skillpouch/current/…` (so an edit in any assistant
//! edits the pouch), and MCP servers, hooks and plugins written into their
//! config files.
//!
//! Where a link can't work, the assistant gets a copy instead: commands in
//! Gemini's TOML format, and files on Windows without Developer Mode. Edits
//! to a copy are pulled back into the pouch before each sync.
//!
//! Something already at a link's place that isn't ours is moved into
//! `~/.skillpouch/backups/<time>/` first; links and copies SkillPouch made
//! that the pouch no longer needs are removed. Assistants whose folder is
//! part of a git checkout are only read. Nothing else is touched.

use crate::bundle::{self, Unpacked};
use crate::error::io;
use crate::hooks::{self, HookFile};
use crate::item::{self, LocalItem};
use crate::mcp::{self, Server};
use crate::plugins::{self, PluginFile};
use crate::translate;
use crate::{Result, SyncError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sp_core::Home;
use sp_core::home::{read_optional, write_private};
use sp_harness::{AgentFormat, CommandFormat, PluginFormat, Target};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// What projection did last time, so it can undo exactly that.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projection {
    /// Link path → where it points.
    #[serde(default)]
    pub links: BTreeMap<PathBuf, PathBuf>,
    /// Config file → the servers SkillPouch wrote there, as written.
    #[serde(default)]
    pub mcp: BTreeMap<PathBuf, BTreeMap<String, Server>>,
    /// Config file → servers that were there at setup and stay this
    /// computer's own (never brought into the pouch).
    #[serde(default)]
    pub ignored: BTreeMap<PathBuf, BTreeSet<String>>,
    /// Items deleted in an assistant while a read-only one (a git checkout)
    /// still has them: not brought back in from there.
    #[serde(default)]
    pub dismissed: BTreeSet<String>,
    /// Copy path → the pouch file it was made from.
    #[serde(default)]
    pub copies: BTreeMap<PathBuf, Copied>,
    /// Hooks file → the entries SkillPouch wrote there.
    #[serde(default)]
    pub hooks: BTreeMap<PathBuf, HookFile>,
    /// Config file → the plugins SkillPouch turned on there.
    #[serde(default)]
    pub plugins: BTreeMap<PathBuf, PluginFile>,
    /// Links and copies of a plugin's parts: never items of their own, so
    /// what's found there isn't brought into the pouch.
    #[serde(default)]
    pub plugin_parts: BTreeSet<PathBuf>,
    /// MCP servers that come from a plugin: an edit to one is undone, not
    /// brought into the pouch.
    #[serde(default)]
    pub plugin_servers: BTreeSet<String>,
}

/// How a copy is made from its pouch file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Render {
    /// Byte for byte.
    Plain,
    /// `COMMAND.md` as a Gemini CLI TOML command.
    TomlCommand,
    /// `AGENT.md` in an assistant's own dialect.
    Agent(AgentFormat),
    /// `COMMAND.md` in an assistant's own dialect.
    Command(CommandFormat),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copied {
    pub from: PathBuf,
    pub render: Render,
    /// BLAKE3 of what was written, to tell our copy from an edited one.
    pub hash: String,
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// The item a pouch file belongs to: `…/agents/<name>/AGENT.md` → `<name>`;
/// a plugin's `…/agents/<name>.md` → `<name>`.
fn item_name(from: &Path) -> String {
    let named = if from.file_name().is_some_and(|file| file == "AGENT.md") {
        from.parent().and_then(Path::file_name)
    } else {
        from.file_stem()
    };
    named
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn render(render: Render, source: &[u8], from: &Path) -> Vec<u8> {
    let text = || String::from_utf8_lossy(source);
    match render {
        Render::Plain => source.to_vec(),
        Render::TomlCommand => translate::command_to(CommandFormat::Toml, &text()).into_bytes(),
        Render::Agent(format) => {
            translate::agent_to(format, &text(), &item_name(from)).into_bytes()
        }
        Render::Command(format) => translate::command_to(format, &text()).into_bytes(),
    }
}

/// What to put back into the pouch file for an edited copy.
fn unrender(render: Render, copy: &[u8], current: &[u8], from: &Path) -> Option<Vec<u8>> {
    if render == Render::Plain {
        return Some(copy.to_vec());
    }
    let (copy, current) = (
        String::from_utf8_lossy(copy),
        String::from_utf8_lossy(current),
    );
    match render {
        Render::Plain => None,
        Render::TomlCommand => translate::command_from(CommandFormat::Toml, &copy, &current),
        Render::Agent(format) => Some(translate::agent_from(
            format,
            &copy,
            &current,
            &item_name(from),
        )),
        Render::Command(format) => translate::command_from(format, &copy, &current),
    }
    .map(String::into_bytes)
}

impl Projection {
    fn path(home: &Home) -> PathBuf {
        home.state_dir().join("projection.json")
    }

    pub fn load(home: &Home) -> Self {
        read_optional(&Self::path(home))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        home.ensure_private_dir(&home.state_dir())?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| SyncError::Invalid(error.to_string()))?;
        Ok(write_private(&Self::path(home), &bytes)?)
    }

    /// MCP servers edited by hand in an assistant's config since SkillPouch
    /// wrote them: `(name, server as it is now)`.
    pub fn edited_servers(&self, targets: &[Target]) -> Vec<(String, Server)> {
        let mut edited = Vec::new();
        for target in targets {
            let Some((path, format)) = &target.mcp else {
                continue;
            };
            let Some(written) = self.mcp.get(path) else {
                continue;
            };
            let current = mcp::read(path, *format);
            for (name, ours) in written {
                if self.plugin_servers.contains(name) {
                    continue;
                }
                if let Some(now) = current.get(name)
                    && !same_server(now, ours)
                {
                    edited.push((name.clone(), now.clone()));
                }
            }
        }
        edited
    }

    /// Servers in a config SkillPouch didn't write.
    pub fn foreign_servers(&self, targets: &[Target]) -> Vec<(String, Server)> {
        let mut found = Vec::new();
        for target in targets {
            let Some((path, format)) = &target.mcp else {
                continue;
            };
            let ours = self.mcp.get(path);
            for (name, server) in mcp::read(path, *format) {
                if !ours.is_some_and(|ours| ours.contains_key(&name)) {
                    found.push((name, server));
                }
            }
        }
        found
    }
}

impl Projection {
    /// Puts edits made to copies back into their pouch files, so the sync
    /// sends them. Returns the copies that were pulled.
    pub fn pull_copies(&mut self) -> Result<Vec<PathBuf>> {
        let mut pulled = Vec::new();
        for (path, copied) in &mut self.copies {
            let Ok(now) = fs::read(path) else {
                continue;
            };
            let current_hash = hash(&now);
            if current_hash == copied.hash {
                continue;
            }
            // The item left the pouch meanwhile: the copy is the user's now.
            let Ok(current) = fs::read(&copied.from) else {
                continue;
            };
            let Some(bytes) = unrender(copied.render, &now, &current, &copied.from) else {
                continue;
            };
            if bytes != current {
                item::write_file(&copied.from, &item::File::new(bytes))?;
            }
            copied.hash = current_hash;
            pulled.push(path.clone());
        }
        Ok(pulled)
    }
}

/// Configs drop what they don't know (`description`): compare what runs.
fn same_server(a: &Value, b: &Value) -> bool {
    let strip = |value: &Value| {
        let mut value = value.clone();
        if let Some(fields) = value.as_object_mut() {
            fields.remove("description");
        }
        value
    };
    strip(a) == strip(b)
}

/// What one projection did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Projected {
    pub linked: usize,
    /// Moved aside to make room for a link.
    pub backed_up: Vec<PathBuf>,
    /// Places left alone, with why.
    pub problems: Vec<String>,
}

/// Where the links point: `~/.skillpouch/current`, itself a link to the
/// active pouch's folder, so switching pouches swaps one link.
pub fn current(home: &Home) -> PathBuf {
    home.root().join("current")
}

/// Points `current` at `pouch_dir`.
pub fn set_current(home: &Home, pouch_dir: &Path) -> Result<()> {
    let link = current(home);
    if fs::read_link(&link).is_ok_and(|to| to == pouch_dir) {
        return Ok(());
    }
    let staged = home.root().join("current.new");
    let _ = fs::remove_file(&staged);
    symlink(pouch_dir, &staged, true)?;
    // Windows can't rename over a folder link; the gap is a moment long.
    if cfg!(windows) {
        let _ = remove_link(&link);
    }
    fs::rename(&staged, &link).map_err(io(&link))
}

/// Links `items` into every target and writes their MCP servers, hooks and
/// plugins.
pub fn project(
    home: &Home,
    items: &BTreeMap<String, LocalItem>,
    targets: &[Target],
    backups: &Path,
    previous: &Projection,
) -> Result<(Projection, Projected)> {
    let base = current(home);
    let file_links = file_links_work(home);
    let mut wanted: BTreeMap<PathBuf, Place> = BTreeMap::new();
    let mut servers: BTreeMap<String, Server> = BTreeMap::new();
    let mut report = Projected::default();
    let instructions = items
        .values()
        .filter(|item| item.kind == "instructions")
        .min_by_key(|item| item.name != item::INSTRUCTIONS);
    let file = |to: PathBuf| file_place(file_links, to);
    let unpacked: Vec<Unpacked> = items
        .values()
        .filter(|item| item.kind == "plugin")
        .filter_map(|item| bundle::unpack(item, &base.join("plugins").join(&item.name)))
        .collect();
    let mut part_places: Vec<(PathBuf, Place, &String)> = Vec::new();
    let writable: Vec<&Target> = targets
        .iter()
        .filter(|target| match &target.read_only {
            Some(reason) => {
                report.problems.push(format!("{}: {reason}.", target.name));
                false
            }
            None => true,
        })
        .collect();
    for target in &writable {
        if let (Some(item), Some(path)) = (instructions, &target.instructions) {
            wanted.insert(path.clone(), file(source(&base, item, "INSTRUCTIONS.md")));
        }
        for item in items.values() {
            if !matches!(item.kind.as_str(), "skill") && !item.files.contains_key(main_of(item)) {
                continue;
            }
            let place = match item.kind.as_str() {
                "skill" => target.skills.as_ref().map(|dir| {
                    (
                        dir.join(&item.name),
                        Place::Link {
                            to: base.join("skills").join(&item.name),
                            dir: true,
                        },
                    )
                }),
                "agent" => target.agents.as_ref().map(|agents| {
                    agent_place(
                        file_links,
                        agents,
                        &item.name,
                        source(&base, item, "AGENT.md"),
                    )
                }),
                "command" => target.commands.as_ref().map(|commands| {
                    let from = source(&base, item, "COMMAND.md");
                    command_place(file_links, commands, &item.name, from)
                }),
                _ => None,
            };
            if let Some((path, place)) = place {
                wanted.insert(path, place);
            }
        }
        if installs_plugins(target) {
            continue;
        }
        for plugin in &unpacked {
            let mut parts: Vec<(PathBuf, Place)> = Vec::new();
            if let Some(dir) = &target.skills {
                parts.extend(plugin.skills.iter().map(|(name, to)| {
                    let place = Place::Link {
                        to: to.clone(),
                        dir: true,
                    };
                    (dir.join(name), place)
                }));
            }
            if let Some(agents) = &target.agents {
                parts.extend(
                    plugin
                        .agents
                        .iter()
                        .map(|(name, from)| agent_place(file_links, agents, name, from.clone())),
                );
            }
            if let Some(commands) = &target.commands {
                parts.extend(
                    plugin.commands.iter().map(|(name, from)| {
                        command_place(file_links, commands, name, from.clone())
                    }),
                );
            }
            part_places.extend(
                parts
                    .into_iter()
                    .map(|(path, place)| (path, place, &plugin.plugin)),
            );
        }
    }
    // The pouch's own items win over a plugin's of the same name, and the
    // first plugin over the next.
    let mut plugin_parts = BTreeSet::new();
    for (path, place, plugin) in part_places {
        match wanted.get(&path) {
            None => {
                plugin_parts.insert(path.clone());
                wanted.insert(path, place);
            }
            Some(there) if *there == place => {}
            Some(_) => report.problems.push(format!(
                "{}: left out of plugin {plugin}, something else of that name is already there.",
                path.display()
            )),
        }
    }
    for item in items.values().filter(|item| item.kind == "mcp_server") {
        if let Some(file) = item.files.get("server.json")
            && let Ok(server) = mcp::parse_server(&file.bytes)
        {
            servers.insert(item.name.clone(), server);
        }
    }
    let mut with_plugins = servers.clone();
    let mut plugin_servers = BTreeSet::new();
    for plugin in &unpacked {
        for (name, server) in &plugin.servers {
            if with_plugins.contains_key(name) {
                report.problems.push(format!(
                    "MCP server {name}: left out of plugin {}, another server has that name.",
                    plugin.plugin
                ));
                continue;
            }
            with_plugins.insert(name.clone(), server.clone());
            plugin_servers.insert(name.clone());
        }
    }

    let mut next = Projection {
        plugin_parts,
        plugin_servers,
        ..Projection::default()
    };
    for (path, place) in &wanted {
        let placed = match place {
            Place::Link { to, dir } => place_link(home, path, to, *dir, backups).inspect(|_| {
                next.links.insert(path.clone(), to.clone());
            }),
            Place::Copy { from, render } => place_copy(
                home,
                path,
                from,
                *render,
                previous.copies.get(path),
                backups,
            )
            .map(|(backup, copied)| {
                if let Some(copied) = copied {
                    next.copies.insert(path.clone(), copied);
                }
                backup
            }),
        };
        match placed {
            Ok(backup) => {
                report.linked += 1;
                report.backed_up.extend(backup);
            }
            Err(error) => report.problems.push(error.to_string()),
        }
    }
    for (path, to) in &previous.links {
        if wanted.contains_key(path) {
            continue;
        }
        // Only remove it if it's still the link we made.
        if fs::read_link(path).is_ok_and(|now| &now == to) {
            let _ = remove_link(path);
        }
    }
    for (path, copied) in &previous.copies {
        // Only remove it if it's still the copy we made.
        if !wanted.contains_key(path) && fs::read(path).is_ok_and(|now| hash(&now) == copied.hash) {
            let _ = fs::remove_file(path);
        }
    }
    for target in &writable {
        let Some((path, format)) = &target.mcp else {
            continue;
        };
        let managed: BTreeSet<String> = previous
            .mcp
            .get(path)
            .map(|written| written.keys().cloned().collect())
            .unwrap_or_default();
        let servers = if installs_plugins(target) {
            &servers
        } else {
            &with_plugins
        };
        let written = mcp::write(path, *format, servers, &managed)?;
        if let Some(problem) = written.skipped {
            report.problems.push(problem);
            if let Some(old) = previous.mcp.get(path) {
                next.mcp.insert(path.clone(), old.clone());
            }
            continue;
        }
        let as_written: BTreeMap<String, Server> = mcp::read(path, *format)
            .into_iter()
            .filter(|(name, _)| written.managed.contains(name))
            .collect();
        if !as_written.is_empty() {
            next.mcp.insert(path.clone(), as_written);
        }
    }

    let hooks_to_run: Vec<hooks::Hook> = items
        .values()
        .filter(|item| item.kind == "hook")
        .filter_map(|item| {
            let hook = hooks::parse_hook(&item.files.get("hook.json")?.bytes).ok()?;
            Some(hooks::Hook {
                name: item.name.clone(),
                ..hook
            })
        })
        .collect();
    let mut with_plugin_hooks = hooks_to_run.clone();
    for hook in unpacked.iter().flat_map(|plugin| &plugin.hooks) {
        if !with_plugin_hooks.iter().any(|have| have.same_action(hook)) {
            with_plugin_hooks.push(hook.clone());
        }
    }
    for target in &writable {
        let Some((path, format)) = &target.hooks else {
            continue;
        };
        let hooks_to_run = if installs_plugins(target) {
            &hooks_to_run
        } else {
            &with_plugin_hooks
        };
        match hooks::write(path, *format, hooks_to_run, previous.hooks.get(path))? {
            Ok(record) => {
                next.hooks.insert(path.clone(), record);
            }
            Err(problem) => {
                report.problems.push(problem);
                if let Some(old) = previous.hooks.get(path) {
                    next.hooks.insert(path.clone(), old.clone());
                }
            }
        }
    }

    let plugin_items: Vec<&LocalItem> = items
        .values()
        .filter(|item| item.kind == "plugin")
        .collect();
    let marketplace = base.join("plugins");
    if writable
        .iter()
        .any(|target| matches!(target.plugins, Some((_, PluginFormat::Claude))))
    {
        plugins::write_marketplace(&marketplace, &plugin_items)?;
    }
    for target in &writable {
        let Some((path, format)) = &target.plugins else {
            continue;
        };
        match plugins::write(
            path,
            *format,
            &plugin_items,
            &marketplace,
            previous.plugins.get(path),
        )? {
            Ok(record) => {
                next.plugins.insert(path.clone(), record);
            }
            Err(problem) => {
                report.problems.push(problem);
                if let Some(old) = previous.plugins.get(path) {
                    next.plugins.insert(path.clone(), old.clone());
                }
            }
        }
    }
    Ok((next, report))
}

/// Claude Code installs plugin folders itself; everyone else gets their
/// parts.
fn installs_plugins(target: &Target) -> bool {
    matches!(target.plugins, Some((_, PluginFormat::Claude)))
}

fn file_place(file_links: bool, to: PathBuf) -> Place {
    if file_links {
        Place::Link { to, dir: false }
    } else {
        Place::Copy {
            from: to,
            render: Render::Plain,
        }
    }
}

fn agent_place(
    file_links: bool,
    (dir, format): &(PathBuf, AgentFormat),
    name: &str,
    from: PathBuf,
) -> (PathBuf, Place) {
    let path = dir.join(format!("{name}.md"));
    match format {
        AgentFormat::Claude => (path, file_place(file_links, from)),
        format => (
            path,
            Place::Copy {
                from,
                render: Render::Agent(*format),
            },
        ),
    }
}

fn command_place(
    file_links: bool,
    (dir, format): &(PathBuf, CommandFormat),
    name: &str,
    from: PathBuf,
) -> (PathBuf, Place) {
    let path = dir.join(format!("{name}.{}", format.extension()));
    let render = match format {
        CommandFormat::Markdown => return (path, file_place(file_links, from)),
        CommandFormat::Toml => Render::TomlCommand,
        format => Render::Command(*format),
    };
    (path, Place::Copy { from, render })
}

/// What goes at one place in an assistant's folder.
#[derive(PartialEq)]
enum Place {
    Link { to: PathBuf, dir: bool },
    Copy { from: PathBuf, render: Render },
}

/// Writes the copy of `from` at `path`; returns where an existing file went
/// and the record of the copy (`None` when the pouch file is missing).
fn place_copy(
    home: &Home,
    path: &Path,
    from: &Path,
    render_as: Render,
    previous: Option<&Copied>,
    backups: &Path,
) -> Result<(Option<PathBuf>, Option<Copied>)> {
    let Ok(source) = fs::read(from) else {
        return Ok((None, None));
    };
    let bytes = render(render_as, &source, from);
    let copied = Copied {
        from: from.to_path_buf(),
        render: render_as,
        hash: hash(&bytes),
    };
    let mut backup = None;
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if fs::read_link(path).is_ok_and(|to| to.starts_with(home.root())) {
                remove_link(path).map_err(io(path))?;
            } else {
                backup = Some(back_up(path, backups)?);
            }
        }
        Ok(_) => {
            let now = fs::read(path).unwrap_or_default();
            if now == bytes {
                return Ok((None, Some(copied)));
            }
            // Ours and unchanged since: just rewrite it. Anything else is
            // the user's and goes to the backups first.
            if !previous.is_some_and(|previous| previous.hash == hash(&now)) {
                backup = Some(back_up(path, backups)?);
            }
        }
        Err(_) => {}
    }
    item::write_file(path, &item::File::new(bytes))?;
    Ok((backup, Some(copied)))
}

/// Whether this computer lets SkillPouch link single files (Windows only
/// does with Developer Mode on); checked once per run.
fn file_links_work(home: &Home) -> bool {
    if cfg!(unix) {
        return true;
    }
    let probe = home.run_dir().join("link-probe");
    let _ = fs::create_dir_all(home.run_dir());
    let _ = fs::remove_file(&probe);
    let works = symlink(&home.config_file(), &probe, false).is_ok();
    let _ = fs::remove_file(&probe);
    works
}

fn remove_link(path: &Path) -> std::io::Result<()> {
    // Windows junctions and directory links are removed as directories.
    fs::remove_file(path).or_else(|_| fs::remove_dir(path))
}

fn main_of(item: &LocalItem) -> &'static str {
    item::main_file(&item.kind).unwrap_or("")
}

fn source(base: &Path, item: &LocalItem, file: &str) -> PathBuf {
    let folder = item::folder_of(&item.kind).unwrap_or("");
    base.join(folder).join(&item.name).join(file)
}

/// Puts a link to `to` at `path`; returns where an existing file went.
fn place_link(
    home: &Home,
    path: &Path,
    to: &Path,
    dir: bool,
    backups: &Path,
) -> Result<Option<PathBuf>> {
    if let Ok(now) = fs::read_link(path) {
        if now == to {
            return Ok(None);
        }
        // An old link of ours (another pouch or layout): just replace it.
        if now.starts_with(home.root()) {
            remove_link(path).map_err(io(path))?;
            symlink(to, path, dir)?;
            return Ok(None);
        }
    }
    let mut backup = None;
    if fs::symlink_metadata(path).is_ok() {
        let aside = backup_path(path, backups);
        if let Some(parent) = aside.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        fs::rename(path, &aside).map_err(io(path))?;
        backup = Some(aside);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    symlink(to, path, dir)?;
    Ok(backup)
}

/// `backups/<time>/home/me/.claude/CLAUDE.md` for `/home/me/.claude/CLAUDE.md`.
pub fn backup_path(path: &Path, backups: &Path) -> PathBuf {
    let relative: PathBuf = path
        .components()
        .filter(|part| matches!(part, std::path::Component::Normal(_)))
        .collect();
    backups.join(relative)
}

/// Moves `path` into the backups; returns where it went.
pub fn back_up(path: &Path, backups: &Path) -> Result<PathBuf> {
    let aside = backup_path(path, backups);
    if let Some(parent) = aside.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    fs::rename(path, &aside).map_err(io(path))?;
    Ok(aside)
}

#[cfg(unix)]
fn symlink(to: &Path, path: &Path, _dir: bool) -> Result<()> {
    std::os::unix::fs::symlink(to, path).map_err(io(path))
}

/// Folders: a symbolic link, else a junction (which needs no Developer
/// Mode). Files: a symbolic link; without Developer Mode they are copied
/// instead (`file_links_work`).
#[cfg(windows)]
fn symlink(to: &Path, path: &Path, dir: bool) -> Result<()> {
    if !dir {
        return std::os::windows::fs::symlink_file(to, path).map_err(io(path));
    }
    if std::os::windows::fs::symlink_dir(to, path).is_ok() {
        return Ok(());
    }
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(path)
        .arg(to)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(io(path))?;
    if status.success() {
        Ok(())
    } else {
        Err(SyncError::Invalid(format!(
            "{}: couldn't link this folder",
            path.display()
        )))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::item::{File, Files};
    use sp_harness::McpFormat;

    fn item(kind: &str, name: &str, files: &[(&str, &str)]) -> (String, LocalItem) {
        let files: Files = files
            .iter()
            .map(|(path, text)| (path.to_string(), File::new(*text)))
            .collect();
        (
            item::key(kind, name),
            LocalItem {
                kind: kind.into(),
                name: name.into(),
                files,
            },
        )
    }

    fn target(root: &Path, json: &Path) -> Target {
        Target {
            id: "claude-code",
            name: "Claude Code",
            root: root.to_path_buf(),
            settings: root.to_path_buf(),
            instructions: Some(root.join("CLAUDE.md")),
            skills: Some(root.join("skills")),
            agents: Some((root.join("agents"), sp_harness::AgentFormat::Claude)),
            commands: Some((root.join("commands"), CommandFormat::Markdown)),
            mcp: Some((json.to_path_buf(), McpFormat::Claude)),
            hooks: None,
            plugins: None,
            read_only: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn hooks_and_plugins_reach_every_assistant() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = crate::item::PouchDir::new(&pouch);
        let claude = tmp.path().join(".claude");
        let gemini = tmp.path().join(".gemini");
        fs::create_dir_all(&claude).map_err(io(&claude))?;
        let settings = claude.join("settings.json");
        let items: BTreeMap<_, _> = [
            item(
                "hook",
                "format",
                &[(
                    "hook.json",
                    r#"{"name":"format","event":"PostToolUse","matcher":"Edit|Write","command":"prettier --write ."}"#,
                )],
            ),
            item(
                "plugin",
                "lint",
                &[(".claude-plugin/plugin.json", r#"{"name":"lint"}"#), ("commands/lint.md", "Lint")],
            ),
        ]
        .into_iter()
        .collect();
        for item in items.values() {
            dir.write_item(&item.kind, &item.name, &item.files)?;
        }
        set_current(&home, &pouch)?;
        let mut claude_target = target(&claude, &tmp.path().join(".claude.json"));
        claude_target.hooks = Some((settings.clone(), sp_harness::HookFormat::Claude));
        claude_target.plugins = Some((settings.clone(), PluginFormat::Claude));
        let gemini_target = Target {
            id: "gemini-cli",
            name: "Gemini CLI",
            root: gemini.clone(),
            settings: gemini.clone(),
            instructions: None,
            skills: None,
            agents: None,
            commands: None,
            mcp: None,
            hooks: Some((gemini.join("settings.json"), sp_harness::HookFormat::Gemini)),
            plugins: None,
            read_only: None,
        };
        let targets = [claude_target, gemini_target];
        let backups = home.root().join("backups/t1");

        let (second, _) = project(&home, &items, &targets, &backups, &Projection::default())?;
        let value: Value =
            serde_json::from_str(&fs::read_to_string(&settings).map_err(io(&settings))?)
                .map_err(|e| SyncError::Invalid(e.to_string()))?;
        assert_eq!(
            value["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "prettier --write ."
        );
        assert_eq!(value["enabledPlugins"]["lint@skillpouch"], true);
        let gemini_settings = gemini.join("settings.json");
        let value: Value = serde_json::from_str(
            &fs::read_to_string(&gemini_settings).map_err(io(&gemini_settings))?,
        )
        .map_err(|e| SyncError::Invalid(e.to_string()))?;
        assert_eq!(value["hooks"]["AfterTool"][0]["matcher"], "Edit|Write");
        assert!(
            pouch
                .join("plugins/.claude-plugin/marketplace.json")
                .exists()
        );

        // Gone from the pouch: gone from the assistants.
        let (_, _) = project(&home, &BTreeMap::new(), &targets, &backups, &second)?;
        let value: Value =
            serde_json::from_str(&fs::read_to_string(&settings).map_err(io(&settings))?)
                .map_err(|e| SyncError::Invalid(e.to_string()))?;
        assert_eq!(value["enabledPlugins"], serde_json::json!({}));
        assert!(value["hooks"].get("PostToolUse").is_none(), "{value}");
        Ok(())
    }

    fn json_at(path: &Path) -> Result<Value> {
        serde_json::from_str(&fs::read_to_string(path).map_err(io(path))?)
            .map_err(|e| SyncError::Invalid(e.to_string()))
    }

    #[cfg(unix)]
    #[test]
    fn a_plugin_folder_reaches_assistants_without_plugins_in_parts() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = crate::item::PouchDir::new(&pouch);
        let claude = tmp.path().join(".claude");
        let opencode = tmp.path().join(".config/opencode");
        let gemini = tmp.path().join(".gemini");
        for folder in [&claude, &opencode, &gemini] {
            fs::create_dir_all(folder).map_err(io(folder))?;
        }
        let items: BTreeMap<_, _> = [
            item(
                "plugin",
                "ponytail",
                &[
                    (
                        ".claude-plugin/plugin.json",
                        r#"{"name":"ponytail","hooks":"./hooks/claude.json"}"#,
                    ),
                    ("skills/lazy/SKILL.md", "---\nname: lazy\n---\nfrom the plugin"),
                    ("skills/terse/SKILL.md", "---\nname: terse\n---\n"),
                    (
                        "agents/reviewer.md",
                        "---\nname: reviewer\ndescription: Reviews\ntools: Read, Grep\n---\nReview.",
                    ),
                    ("commands/simplify.md", "---\ndescription: Simplify\n---\nSimplify $ARGUMENTS"),
                    (
                        "hooks/claude.json",
                        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"node \"${CLAUDE_PLUGIN_ROOT}/hooks/start.js\"","timeout":5}]}]}}"#,
                    ),
                    ("hooks/start.js", "console.log('hi')"),
                    (".mcp.json", r#"{"mcpServers":{"docs":{"type":"http","url":"https://x/mcp"}}}"#),
                ],
            ),
            item("skill", "lazy", &[("SKILL.md", "---\nname: lazy\n---\nthe pouch's own")]),
        ]
        .into_iter()
        .collect();
        for item in items.values() {
            dir.write_item(&item.kind, &item.name, &item.files)?;
        }
        set_current(&home, &pouch)?;
        let settings = claude.join("settings.json");
        let mut claude_target = target(&claude, &tmp.path().join(".claude.json"));
        claude_target.hooks = Some((settings.clone(), sp_harness::HookFormat::Claude));
        claude_target.plugins = Some((settings.clone(), PluginFormat::Claude));
        let opencode_json = opencode.join("opencode.json");
        let opencode_target = Target {
            id: "opencode",
            name: "OpenCode",
            root: opencode.clone(),
            settings: opencode.clone(),
            instructions: None,
            skills: Some(opencode.join("skills")),
            agents: Some((opencode.join("agents"), AgentFormat::OpenCode)),
            commands: Some((opencode.join("commands"), CommandFormat::OpenCode)),
            mcp: Some((opencode_json.clone(), McpFormat::OpenCode)),
            hooks: None,
            plugins: Some((opencode_json.clone(), PluginFormat::OpenCode)),
            read_only: None,
        };
        let gemini_settings = gemini.join("settings.json");
        let gemini_target = Target {
            id: "gemini-cli",
            name: "Gemini CLI",
            root: gemini.clone(),
            settings: gemini.clone(),
            instructions: None,
            skills: None,
            agents: None,
            commands: Some((gemini.join("commands"), CommandFormat::Toml)),
            mcp: Some((gemini_settings.clone(), McpFormat::Gemini)),
            hooks: Some((gemini_settings.clone(), sp_harness::HookFormat::Gemini)),
            plugins: None,
            read_only: None,
        };
        let targets = [claude_target, opencode_target, gemini_target];
        let backups = home.root().join("backups/t1");
        let (first, report) = project(&home, &items, &targets, &backups, &Projection::default())?;
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert!(
            report.problems[0].contains("skills/lazy: left out of plugin ponytail"),
            "{:?}",
            report.problems
        );
        assert!(first.plugin_parts.contains(&opencode.join("skills/terse")));
        assert!(
            first
                .plugin_parts
                .contains(&gemini.join("commands/simplify.toml"))
        );
        assert!(!first.plugin_parts.contains(&opencode.join("skills/lazy")));
        assert_eq!(first.plugin_servers, BTreeSet::from(["docs".to_string()]));

        // Claude Code installs the plugin itself and gets none of its parts.
        assert_eq!(
            json_at(&settings)?["enabledPlugins"]["ponytail@skillpouch"],
            true
        );
        assert!(json_at(&settings)?.get("hooks").is_none());
        assert!(!claude.join("skills/terse").exists());
        assert!(!claude.join("agents/reviewer.md").exists());

        // OpenCode gets every part in its own dialect.
        let root = current(&home).join("plugins/ponytail");
        assert_eq!(
            fs::read_link(opencode.join("skills/terse")).map_err(io(&opencode))?,
            root.join("skills/terse")
        );
        // A pouch skill of the same name wins.
        assert_eq!(
            fs::read_link(opencode.join("skills/lazy")).map_err(io(&opencode))?,
            current(&home).join("skills/lazy")
        );
        let agent = opencode.join("agents/reviewer.md");
        let agent = fs::read_to_string(&agent).map_err(io(&agent))?;
        assert!(agent.contains("mode: subagent"), "{agent}");
        assert!(opencode.join("commands/simplify.md").exists());
        assert_eq!(
            mcp::read(&opencode_json, McpFormat::OpenCode)["docs"]["url"],
            "https://x/mcp"
        );

        // Gemini CLI: a TOML command, the MCP server and the hook with the
        // plugin's folder filled in.
        let toml = gemini.join("commands/simplify.toml");
        let toml = fs::read_to_string(&toml).map_err(io(&toml))?;
        assert!(toml.contains("{{args}}"), "{toml}");
        let value = json_at(&gemini_settings)?;
        assert_eq!(value["mcpServers"]["docs"]["httpUrl"], "https://x/mcp");
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert_eq!(
            command,
            format!(
                "export CLAUDE_PLUGIN_ROOT='{}'; node \"${{CLAUDE_PLUGIN_ROOT}}/hooks/start.js\"",
                root.display()
            )
        );

        // A plugin's server edited by hand isn't the user's own server.
        let text = fs::read_to_string(&gemini_settings).map_err(io(&gemini_settings))?;
        fs::write(
            &gemini_settings,
            text.replace("https://x/mcp", "https://y/mcp"),
        )
        .map_err(io(&gemini_settings))?;
        assert!(first.edited_servers(&targets).is_empty());

        // Taken out of the pouch: every part goes too.
        let rest: BTreeMap<_, _> = items
            .into_iter()
            .filter(|(_, item)| item.kind != "plugin")
            .collect();
        let (_, report) = project(&home, &rest, &targets, &backups, &first)?;
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert!(fs::symlink_metadata(opencode.join("skills/terse")).is_err());
        assert!(!opencode.join("agents/reviewer.md").exists());
        assert!(!gemini.join("commands/simplify.toml").exists());
        assert!(mcp::read(&opencode_json, McpFormat::OpenCode).is_empty());
        let value = json_at(&gemini_settings)?;
        assert!(value["mcpServers"].get("docs").is_none(), "{value}");
        assert!(value["hooks"].get("SessionStart").is_none(), "{value}");
        Ok(())
    }

    // Droid ignores marketplaces it didn't add itself (and rejects a
    // `directory` source), so Factory gets a plugin folder's parts.
    #[cfg(unix)]
    #[test]
    fn factory_gets_a_plugin_folders_parts_and_keeps_its_references() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = crate::item::PouchDir::new(&pouch);
        let factory = tmp.path().join(".factory");
        fs::create_dir_all(&factory).map_err(io(&factory))?;
        let items: BTreeMap<_, _> = [
            item(
                "plugin",
                "kit",
                &[
                    (".claude-plugin/plugin.json", r#"{"name":"kit"}"#),
                    ("skills/tidy/SKILL.md", "---\nname: tidy\n---\n"),
                    (
                        "hooks/hooks.json",
                        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"sh \"${CLAUDE_PLUGIN_ROOT}/mark.sh\""}]}]}}"#,
                    ),
                    (".mcp.json", r#"{"mcpServers":{"kit":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/mcp.mjs"]}}}"#),
                ],
            ),
            item(
                "plugin",
                "review",
                &[(
                    "plugin.json",
                    r#"{"name":"review","source":"review@acme","marketplace":"acme/plugins"}"#,
                )],
            ),
        ]
        .into_iter()
        .collect();
        for item in items.values() {
            dir.write_item(&item.kind, &item.name, &item.files)?;
        }
        set_current(&home, &pouch)?;
        let settings = factory.join("settings.json");
        let mcp_json = factory.join("mcp.json");
        let factory_target = Target {
            id: "factory",
            name: "Factory",
            root: factory.clone(),
            settings: factory.clone(),
            instructions: None,
            skills: Some(factory.join("skills")),
            agents: None,
            commands: None,
            mcp: Some((mcp_json.clone(), McpFormat::Claude)),
            hooks: Some((settings.clone(), sp_harness::HookFormat::Claude)),
            plugins: Some((settings.clone(), PluginFormat::Factory)),
            read_only: None,
        };
        let backups = home.root().join("backups/t1");
        let (first, report) = project(
            &home,
            &items,
            &[factory_target],
            &backups,
            &Projection::default(),
        )?;
        assert!(report.problems.is_empty(), "{:?}", report.problems);

        let value = json_at(&settings)?;
        assert_eq!(
            value["enabledPlugins"],
            serde_json::json!({ "review@acme": true })
        );
        assert_eq!(
            value["extraKnownMarketplaces"],
            serde_json::json!({ "acme": { "source": { "source": "github", "repo": "acme/plugins" } } })
        );
        let root = current(&home).join("plugins/kit");
        assert_eq!(
            fs::read_link(factory.join("skills/tidy")).map_err(io(&factory))?,
            root.join("skills/tidy")
        );
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            command.starts_with("export CLAUDE_PLUGIN_ROOT="),
            "{command}"
        );
        assert_eq!(
            mcp::read(&mcp_json, McpFormat::Claude)["kit"]["args"][0],
            root.join("mcp.mjs").to_string_lossy().as_ref()
        );
        assert!(first.plugin_servers.contains("kit"));
        assert!(!pouch.join("plugins/.claude-plugin").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn links_everything_backs_up_what_was_there_and_cleans_up() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = crate::item::PouchDir::new(&pouch);
        let claude = tmp.path().join(".claude");
        let json = tmp.path().join(".claude.json");
        fs::create_dir_all(&claude).map_err(io(&claude))?;
        fs::write(claude.join("CLAUDE.md"), "my old rules").map_err(io(&claude))?;
        fs::write(&json, r#"{"mcpServers":{"mine":{"command":"x"}}}"#).map_err(io(&json))?;

        let items: BTreeMap<_, _> = [
            item(
                "instructions",
                "instructions",
                &[("INSTRUCTIONS.md", "# Rules")],
            ),
            item("skill", "pdf", &[("SKILL.md", "# PDF")]),
            item("agent", "reviewer", &[("AGENT.md", "review")]),
            item(
                "mcp_server",
                "gh",
                &[(
                    "server.json",
                    r#"{"name":"gh","command":"npx","args":[],"env":{}}"#,
                )],
            ),
        ]
        .into_iter()
        .collect();
        for item in items.values() {
            dir.write_item(&item.kind, &item.name, &item.files)?;
        }
        set_current(&home, &pouch)?;
        let targets = [target(&claude, &json)];
        let backups = home.root().join("backups/t1");
        let (projection, report) =
            project(&home, &items, &targets, &backups, &Projection::default())?;
        assert_eq!(report.linked, 3, "{report:?}");
        assert_eq!(report.backed_up.len(), 1);
        assert_eq!(
            fs::read_to_string(claude.join("CLAUDE.md")).map_err(io(&claude))?,
            "# Rules"
        );
        assert_eq!(
            fs::read_to_string(claude.join("skills/pdf/SKILL.md")).map_err(io(&claude))?,
            "# PDF"
        );
        assert_eq!(
            fs::read_to_string(&report.backed_up[0]).map_err(io(&claude))?,
            "my old rules"
        );
        let config: Value = serde_json::from_str(&fs::read_to_string(&json).map_err(io(&json))?)
            .unwrap_or_default();
        assert_eq!(config["mcpServers"]["gh"]["command"], "npx");
        assert!(config["mcpServers"]["mine"].is_object());
        assert!(
            projection
                .foreign_servers(&targets)
                .iter()
                .any(|(n, _)| n == "mine")
        );
        assert!(projection.edited_servers(&targets).is_empty());

        // Editing through the link edits the pouch.
        fs::write(claude.join("skills/pdf/SKILL.md"), "# PDF v2").map_err(io(&claude))?;
        assert_eq!(
            fs::read_to_string(pouch.join("skills/pdf/SKILL.md")).map_err(io(&pouch))?,
            "# PDF v2"
        );

        // The skill and the server leave the pouch: their link and entry go too.
        let fewer: BTreeMap<_, _> = items
            .into_iter()
            .filter(|(_, item)| item.kind != "skill" && item.kind != "mcp_server")
            .collect();
        let (_, report) = project(&home, &fewer, &targets, &backups, &projection)?;
        assert!(report.problems.is_empty(), "{report:?}");
        assert!(fs::symlink_metadata(claude.join("skills/pdf")).is_err());
        let config: Value = serde_json::from_str(&fs::read_to_string(&json).map_err(io(&json))?)
            .unwrap_or_default();
        assert!(config["mcpServers"].get("gh").is_none());
        assert!(config["mcpServers"]["mine"].is_object());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_and_gemini_cli_share_one_instructions_link() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = crate::item::PouchDir::new(&pouch);
        let gemini = tmp.path().join(".gemini");
        fs::create_dir_all(gemini.join("antigravity")).map_err(io(&gemini))?;
        let items: BTreeMap<_, _> = [
            item(
                "instructions",
                "instructions",
                &[("INSTRUCTIONS.md", "# Rules")],
            ),
            item("skill", "pdf", &[("SKILL.md", "# PDF")]),
            item(
                "agent",
                "reviewer",
                &[(
                    "AGENT.md",
                    "---\nname: reviewer\ndescription: Reviews\ntools: Read, Bash\n---\n\nReview.\n",
                )],
            ),
            item(
                "mcp_server",
                "docs",
                &[(
                    "server.json",
                    r#"{"name":"docs","type":"http","url":"https://x/mcp","headers":{}}"#,
                )],
            ),
        ]
        .into_iter()
        .collect();
        for item in items.values() {
            dir.write_item(&item.kind, &item.name, &item.files)?;
        }
        set_current(&home, &pouch)?;
        let places = sp_harness::Places::plain(tmp.path(), home.root());
        let targets: Vec<Target> = sp_harness::targets(&places)
            .into_iter()
            .filter(|target| target.id == "antigravity" || target.id == "gemini-cli")
            .collect();
        assert_eq!(targets.len(), 2);
        let backups = home.root().join("backups/t1");
        let (_, report) = project(&home, &items, &targets, &backups, &Projection::default())?;
        assert!(report.problems.is_empty(), "{report:?}");
        // GEMINI.md once, then a skill and an agent in each assistant.
        assert_eq!(report.linked, 5, "{report:?}");
        let read = |path: &Path| fs::read_to_string(path).map_err(io(path));
        assert_eq!(read(&gemini.join("GEMINI.md"))?, "# Rules");
        let config = gemini.join("config");
        assert_eq!(read(&config.join("skills/pdf/SKILL.md"))?, "# PDF");
        assert!(
            read(&config.join("agents/reviewer.md"))?
                .contains("tools:\n  - view_file\n  - run_command\n")
        );
        let mcp: Value = serde_json::from_str(&read(&config.join("mcp_config.json"))?)
            .map_err(|e| SyncError::Invalid(e.to_string()))?;
        assert_eq!(mcp["mcpServers"]["docs"]["serverUrl"], "https://x/mcp");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn toml_commands_are_copies_whose_edits_come_back() -> Result<()> {
        let tmp = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let home = Home::new(tmp.path().join(".skillpouch"));
        let pouch = home.root().join("pouches/p1");
        let dir = crate::item::PouchDir::new(&pouch);
        let gemini = tmp.path().join(".gemini");
        let items: BTreeMap<_, _> = [item(
            "command",
            "review",
            &[(
                "COMMAND.md",
                "---\ndescription: Review\n---\n\nReview $ARGUMENTS\n",
            )],
        )]
        .into_iter()
        .collect();
        for item in items.values() {
            dir.write_item(&item.kind, &item.name, &item.files)?;
        }
        set_current(&home, &pouch)?;
        let target = Target {
            id: "gemini-cli",
            name: "Gemini CLI",
            root: gemini.clone(),
            settings: gemini.clone(),
            instructions: Some(gemini.join("GEMINI.md")),
            skills: None,
            agents: None,
            commands: Some((gemini.join("commands"), CommandFormat::Toml)),
            mcp: Some((gemini.join("settings.json"), McpFormat::Gemini)),
            hooks: None,
            plugins: None,
            read_only: None,
        };
        let locked = Target {
            id: "opencode",
            name: "OpenCode",
            root: tmp.path().join("oc"),
            settings: tmp.path().join("oc"),
            instructions: Some(tmp.path().join("oc/AGENTS.md")),
            skills: None,
            agents: None,
            commands: Some((tmp.path().join("oc/commands"), CommandFormat::Markdown)),
            mcp: Some((tmp.path().join("oc/opencode.json"), McpFormat::OpenCode)),
            hooks: None,
            plugins: None,
            read_only: Some("part of a git checkout".into()),
        };
        let backups = home.root().join("backups/t1");
        let (mut projection, report) = project(
            &home,
            &items,
            &[target.clone(), locked],
            &backups,
            &Projection::default(),
        )?;
        let copy = gemini.join("commands/review.toml");
        let text = fs::read_to_string(&copy).map_err(io(&copy))?;
        assert!(text.contains("Review {{args}}"), "{text}");
        assert_eq!(projection.copies.len(), 1);
        // The read-only assistant got nothing and says why.
        assert!(!tmp.path().join("oc").exists());
        assert!(
            report.problems.iter().any(|p| p.contains("git checkout")),
            "{report:?}"
        );

        // Nothing edited: nothing to pull.
        assert!(projection.pull_copies()?.is_empty());
        fs::write(
            &copy,
            "description = \"Review\"\nprompt = \"Check {{args}} closely\\n\"\n",
        )
        .map_err(io(&copy))?;
        assert_eq!(projection.pull_copies()?, vec![copy.clone()]);
        let main = pouch.join("commands/review/COMMAND.md");
        assert_eq!(
            fs::read_to_string(&main).map_err(io(&main))?,
            "---\ndescription: Review\n---\n\nCheck $ARGUMENTS closely\n"
        );

        // The command leaves the pouch: our untouched copy goes too.
        let (items, _) = dir.read()?;
        let (next, _) = project(
            &home,
            &items,
            std::slice::from_ref(&target),
            &backups,
            &projection,
        )?;
        assert!(copy.exists());
        let (next, _) = project(&home, &BTreeMap::new(), &[target], &backups, &next)?;
        assert!(!copy.exists());
        assert!(next.copies.is_empty());
        Ok(())
    }
}
