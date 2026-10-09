//! The AI assistants on this computer and what they hold: instructions,
//! skills, agents, commands and MCP servers in each one's global folder.
//! Reading only; nothing here changes a file.
//!
//! Locations follow each assistant's documentation:
//! Claude Code <https://docs.claude.com/en/docs/claude-code/settings>,
//! OpenCode <https://opencode.ai/docs/config/>,
//! Codex <https://github.com/openai/codex/blob/main/docs/config.md>,
//! Gemini CLI <https://github.com/google-gemini/gemini-cli/blob/main/docs/get-started/configuration.md>.

mod config;
mod files;

pub use config::strip_jsonc;
pub use files::{IGNORED_DIRS, ignored_file};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What an item is, in the order the setup screen lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Instructions,
    Skill,
    Agent,
    Command,
    McpServer,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Self::Instructions,
        Self::Skill,
        Self::Agent,
        Self::Command,
        Self::McpServer,
    ];

    /// `1 skill`, `3 skills`: the noun for `count` items.
    pub fn noun(self, count: usize) -> &'static str {
        let one = count == 1;
        match self {
            Self::Instructions if one => "instructions file",
            Self::Instructions => "instructions files",
            Self::Skill if one => "skill",
            Self::Skill => "skills",
            Self::Agent if one => "agent",
            Self::Agent => "agents",
            Self::Command if one => "command",
            Self::Command => "commands",
            Self::McpServer if one => "MCP server",
            Self::McpServer => "MCP servers",
        }
    }

    /// The heading over a list of them.
    pub fn title(self) -> &'static str {
        match self {
            Self::Instructions => "Instructions",
            Self::Skill => "Skills",
            Self::Agent => "Agents",
            Self::Command => "Commands",
            Self::McpServer => "MCP servers",
        }
    }
}

/// One thing an assistant has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    /// The file or folder; for an MCP server, the config file defining it.
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

/// An assistant found on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Harness {
    /// The id the server knows it by (`claude-code`).
    pub id: &'static str,
    pub name: &'static str,
    pub root: PathBuf,
    /// Sorted by kind, then name.
    pub items: Vec<Item>,
}

impl Harness {
    pub fn count(&self, kind: Kind) -> usize {
        self.items.iter().filter(|item| item.kind == kind).count()
    }

    /// `CLAUDE.md, 14 skills, 3 agents and 2 MCP servers`, or `nothing yet`.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for kind in Kind::ALL {
            let count = self.count(kind);
            match (kind, count) {
                (_, 0) => {}
                (Kind::Instructions, 1) => {
                    if let Some(item) = self.items.iter().find(|item| item.kind == kind) {
                        parts.push(item.name.clone());
                    }
                }
                _ => parts.push(format!("{count} {}", kind.noun(count))),
            }
        }
        match parts.split_last() {
            None => "nothing yet".into(),
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        }
    }
}

/// Where each assistant keeps its global setup on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub claude: PathBuf,
    /// Claude Code keeps user MCP servers in `~/.claude.json`, outside its folder.
    pub claude_json: PathBuf,
    pub opencode: PathBuf,
    pub codex: PathBuf,
    pub gemini: PathBuf,
    pub copilot: PathBuf,
    pub cursor: PathBuf,
    pub windsurf: PathBuf,
    pub qwen: PathBuf,
    pub factory: PathBuf,
    pub crush: PathBuf,
    /// `~/.agents`: the shared skills folder several assistants read.
    pub agents: PathBuf,
    /// SkillPouch's own folder: links into it are already synced, not found.
    pub skillpouch: PathBuf,
}

impl Places {
    /// The default places under `home`, ignoring environment variables.
    pub fn plain(home: &Path, skillpouch: &Path) -> Self {
        let config = home.join(".config");
        Self {
            claude: home.join(".claude"),
            claude_json: home.join(".claude.json"),
            opencode: config.join("opencode"),
            codex: home.join(".codex"),
            gemini: home.join(".gemini"),
            copilot: home.join(".copilot"),
            cursor: home.join(".cursor"),
            windsurf: home.join(".codeium/windsurf"),
            qwen: home.join(".qwen"),
            factory: home.join(".factory"),
            crush: config.join("crush"),
            agents: home.join(".agents"),
            skillpouch: skillpouch.to_path_buf(),
        }
    }

    /// The places under `home`, moved by the variables each assistant
    /// honours (`CLAUDE_CONFIG_DIR`, `XDG_CONFIG_HOME`, `CODEX_HOME`,
    /// `COPILOT_HOME`).
    pub fn of(home: &Path, skillpouch: &Path) -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let mut places = Self::plain(home, skillpouch);
        if let Some(dir) = var("CLAUDE_CONFIG_DIR") {
            places.claude_json = dir.join(".claude.json");
            places.claude = dir;
        }
        if let Some(config) = var("XDG_CONFIG_HOME") {
            places.opencode = config.join("opencode");
            places.crush = config.join("crush");
        }
        if let Some(dir) = var("CODEX_HOME") {
            places.codex = dir;
        }
        if let Some(dir) = var("COPILOT_HOME") {
            places.copilot = dir;
        }
        places
    }
}

/// How an assistant's MCP servers are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpFormat {
    /// `mcpServers`, `type` stdio/http/sse (Claude Code, Cursor, Factory).
    Claude,
    /// `mcp`, `local` with a command array, or `remote` (OpenCode).
    OpenCode,
    /// `[mcp_servers]` in TOML (Codex).
    Codex,
    /// `mcpServers`, `httpUrl` for HTTP and `url` for SSE (Gemini CLI, Qwen Code).
    Gemini,
    /// `mcpServers`, `type` local/http/sse with `tools` (Copilot CLI).
    Copilot,
    /// `mcpServers`, `serverUrl` for remote servers (Windsurf).
    Windsurf,
}

impl McpFormat {
    /// The key holding the servers.
    pub fn key(self) -> &'static str {
        match self {
            Self::OpenCode => "mcp",
            Self::Codex => "mcp_servers",
            _ => "mcpServers",
        }
    }

    fn file_format(self) -> config::Format {
        match self {
            Self::Codex => config::Format::Toml,
            _ => config::Format::Jsonc,
        }
    }
}

/// How an assistant stores a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandFormat {
    /// The pouch's own: `<name>.md` with `description`, `argument-hint`,
    /// `allowed-tools`, `model`; arguments as `$ARGUMENTS` (Claude Code).
    Markdown,
    /// `<name>.toml` with `description` and `prompt`, arguments as `{{args}}`
    /// (Gemini CLI, Qwen Code).
    Toml,
    /// Markdown with `description`, `agent`, `model` (`provider/model`).
    OpenCode,
    /// Markdown with `description` and `argument-hint` (Codex prompts,
    /// Factory commands).
    Hinted,
    /// Plain Markdown, no front matter (Cursor).
    Plain,
    /// A workflow: Markdown with `description` only (Windsurf).
    Workflow,
}

impl CommandFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            _ => "md",
        }
    }
}

/// How an assistant stores an agent (a subagent with its own prompt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFormat {
    /// The pouch's own: `name`, `description`, `tools: Read, Grep`, `model`
    /// (Claude Code).
    Claude,
    /// `description`, `mode: subagent`, `tools` as a map of `tool: false`.
    OpenCode,
    /// `name`, `description`, `tools` as a list of `read_file`, … (Gemini
    /// CLI, Qwen Code).
    Gemini,
    /// `name`, `description`, `tools` as aliases: `read`, `edit`, `shell`, …
    Copilot,
    /// `name`, `description`, `model: inherit`, `tools`: `Read`, `Execute`, …
    /// (Factory droids).
    Factory,
}

/// How an assistant stores hooks (commands it runs on its own events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFormat {
    /// `hooks` → event → matcher groups → `{type: "command", command,
    /// timeout}` in seconds (Claude Code, Codex `hooks.json`, Qwen Code,
    /// Factory `settings.json`).
    Claude,
    /// The same groups keyed by event at the top level (Factory `hooks.json`).
    ClaudeRoot,
    /// Claude's groups under Gemini's event names, timeouts in milliseconds
    /// (Gemini CLI).
    Gemini,
    /// `{version: 1, hooks: {preToolUse: [{command, matcher}]}}` (Cursor).
    Cursor,
    /// A hooks file of SkillPouch's own in `~/.copilot/hooks/`, entries
    /// `{type: "command", command, matcher, timeoutSec}` (Copilot CLI).
    Copilot,
    /// `hooks` → `pre_run_command` etc. → `[{command}]` (Windsurf).
    Windsurf,
}

/// How an assistant turns plugins on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginFormat {
    /// `enabledPlugins` and `extraKnownMarketplaces` in `settings.json`
    /// (Claude Code, Factory).
    Claude,
    /// npm packages in the `plugin` list of `opencode.json` (OpenCode).
    OpenCode,
}

/// How one assistant lays out its folder.
struct Layout {
    id: &'static str,
    name: &'static str,
    root: PathBuf,
    instructions: Option<&'static str>,
    /// Skill folders it reads; links go into the first.
    skills: Vec<PathBuf>,
    /// Shared skill folders it also reads: when another assistant here
    /// fills one, this one gets no links of its own (no skill twice).
    also_reads: Vec<PathBuf>,
    agents: &'static [&'static str],
    commands: (&'static [&'static str], CommandFormat),
    agent_format: AgentFormat,
    mcp: Option<(Vec<PathBuf>, McpFormat)>,
    hooks: Option<(PathBuf, HookFormat)>,
    /// The config file plugins are turned on in; OpenCode's is its MCP file.
    plugins: Option<(Option<PathBuf>, PluginFormat)>,
}

/// In the order skills are linked: Claude Code's and Codex's folders are
/// the ones others share.
/// Every assistant SkillPouch knows, as `(id, name)`.
pub fn known() -> Vec<(&'static str, &'static str)> {
    let root = Path::new("/");
    layouts(&Places::plain(root, root))
        .into_iter()
        .map(|layout| (layout.id, layout.name))
        .collect()
}

/// How assistant `id` writes agents and commands.
pub fn formats(id: &str) -> Option<(AgentFormat, CommandFormat)> {
    let root = Path::new("/");
    layouts(&Places::plain(root, root))
        .into_iter()
        .find(|layout| layout.id == id)
        .map(|layout| (layout.agent_format, layout.commands.1))
}

fn layouts(places: &Places) -> Vec<Layout> {
    let claude_skills = places.claude.join("skills");
    let shared_skills = places.agents.join("skills");
    vec![
        Layout {
            id: "claude-code",
            name: "Claude Code",
            root: places.claude.clone(),
            instructions: Some("CLAUDE.md"),
            skills: vec![claude_skills.clone()],
            also_reads: vec![],
            agents: &["agents"],
            commands: (&["commands"], CommandFormat::Markdown),
            agent_format: AgentFormat::Claude,
            mcp: Some((vec![places.claude_json.clone()], McpFormat::Claude)),
            hooks: Some((places.claude.join("settings.json"), HookFormat::Claude)),
            plugins: Some((
                Some(places.claude.join("settings.json")),
                PluginFormat::Claude,
            )),
        },
        Layout {
            id: "codex",
            name: "Codex",
            root: places.codex.clone(),
            instructions: Some("AGENTS.md"),
            skills: vec![shared_skills.clone(), places.codex.join("skills")],
            also_reads: vec![],
            agents: &[],
            commands: (&["prompts"], CommandFormat::Hinted),
            agent_format: AgentFormat::Claude,
            mcp: Some((vec![places.codex.join("config.toml")], McpFormat::Codex)),
            hooks: Some((places.codex.join("hooks.json"), HookFormat::Claude)),
            plugins: None,
        },
        Layout {
            id: "opencode",
            name: "OpenCode",
            root: places.opencode.clone(),
            instructions: Some("AGENTS.md"),
            // The singular folders are the older names; both still load.
            skills: vec![
                places.opencode.join("skills"),
                places.opencode.join("skill"),
            ],
            also_reads: vec![claude_skills.clone(), shared_skills.clone()],
            agents: &["agents", "agent"],
            commands: (&["commands", "command"], CommandFormat::OpenCode),
            agent_format: AgentFormat::OpenCode,
            mcp: Some((
                vec![
                    places.opencode.join("opencode.json"),
                    places.opencode.join("opencode.jsonc"),
                ],
                McpFormat::OpenCode,
            )),
            hooks: None,
            plugins: Some((None, PluginFormat::OpenCode)),
        },
        Layout {
            id: "gemini-cli",
            name: "Gemini CLI",
            root: places.gemini.clone(),
            instructions: Some("GEMINI.md"),
            skills: vec![places.gemini.join("skills")],
            also_reads: vec![],
            agents: &["agents"],
            commands: (&["commands"], CommandFormat::Toml),
            agent_format: AgentFormat::Gemini,
            mcp: Some((vec![places.gemini.join("settings.json")], McpFormat::Gemini)),
            hooks: Some((places.gemini.join("settings.json"), HookFormat::Gemini)),
            plugins: None,
        },
        Layout {
            id: "copilot",
            name: "Copilot CLI",
            root: places.copilot.clone(),
            instructions: Some("copilot-instructions.md"),
            skills: vec![places.copilot.join("skills")],
            also_reads: vec![shared_skills.clone()],
            agents: &["agents"],
            commands: (&[], CommandFormat::Markdown),
            agent_format: AgentFormat::Copilot,
            mcp: Some((
                vec![places.copilot.join("mcp-config.json")],
                McpFormat::Copilot,
            )),
            hooks: Some((
                places.copilot.join("hooks").join("skillpouch.json"),
                HookFormat::Copilot,
            )),
            plugins: None,
        },
        Layout {
            id: "cursor",
            name: "Cursor",
            root: places.cursor.clone(),
            instructions: None,
            skills: vec![places.cursor.join("skills")],
            also_reads: vec![
                claude_skills.clone(),
                shared_skills.clone(),
                places.codex.join("skills"),
            ],
            agents: &[],
            commands: (&["commands"], CommandFormat::Plain),
            agent_format: AgentFormat::Claude,
            mcp: Some((vec![places.cursor.join("mcp.json")], McpFormat::Claude)),
            hooks: Some((places.cursor.join("hooks.json"), HookFormat::Cursor)),
            plugins: None,
        },
        Layout {
            id: "windsurf",
            name: "Windsurf",
            root: places.windsurf.clone(),
            instructions: Some("memories/global_rules.md"),
            skills: vec![],
            also_reads: vec![],
            agents: &[],
            commands: (&["global_workflows"], CommandFormat::Workflow),
            agent_format: AgentFormat::Claude,
            mcp: Some((
                vec![places.windsurf.join("mcp_config.json")],
                McpFormat::Windsurf,
            )),
            hooks: Some((places.windsurf.join("hooks.json"), HookFormat::Windsurf)),
            plugins: None,
        },
        Layout {
            id: "qwen-code",
            name: "Qwen Code",
            root: places.qwen.clone(),
            instructions: Some("QWEN.md"),
            skills: vec![],
            also_reads: vec![],
            agents: &["agents"],
            commands: (&["commands"], CommandFormat::Toml),
            agent_format: AgentFormat::Gemini,
            mcp: Some((vec![places.qwen.join("settings.json")], McpFormat::Gemini)),
            hooks: Some((places.qwen.join("settings.json"), HookFormat::Claude)),
            plugins: None,
        },
        Layout {
            id: "factory",
            name: "Factory Droid",
            root: places.factory.clone(),
            instructions: None,
            skills: vec![places.factory.join("skills")],
            also_reads: vec![shared_skills.clone()],
            agents: &["droids"],
            commands: (&["commands"], CommandFormat::Hinted),
            agent_format: AgentFormat::Factory,
            mcp: Some((vec![places.factory.join("mcp.json")], McpFormat::Claude)),
            // Droid reads `settings.json` hooks only while there's no `hooks.json`.
            hooks: Some(if places.factory.join("hooks.json").exists() {
                (places.factory.join("hooks.json"), HookFormat::ClaudeRoot)
            } else {
                (places.factory.join("settings.json"), HookFormat::Claude)
            }),
            plugins: Some((
                Some(places.factory.join("settings.json")),
                PluginFormat::Claude,
            )),
        },
        Layout {
            id: "crush",
            name: "Crush",
            root: places.crush.clone(),
            instructions: Some("CRUSH.md"),
            skills: vec![places.crush.join("skills")],
            also_reads: vec![claude_skills, shared_skills],
            agents: &[],
            commands: (&[], CommandFormat::Markdown),
            agent_format: AgentFormat::Claude,
            mcp: None,
            hooks: None,
            plugins: None,
        },
    ]
}

/// The config file an assistant's MCP servers go to: the first that
/// exists, else the first listed.
fn mcp_file(files: &[PathBuf]) -> Option<PathBuf> {
    files
        .iter()
        .find(|file| file.exists())
        .or_else(|| files.first())
        .cloned()
}

/// Where one assistant reads each kind, for linking synced items into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    pub root: PathBuf,
    /// The global instructions file (`CLAUDE.md`).
    pub instructions: Option<PathBuf>,
    /// One folder per skill.
    pub skills: Option<PathBuf>,
    /// `<name>.md` per agent.
    pub agents: Option<(PathBuf, AgentFormat)>,
    /// One file per command, in the assistant's format.
    pub commands: Option<(PathBuf, CommandFormat)>,
    pub mcp: Option<(PathBuf, McpFormat)>,
    pub hooks: Option<(PathBuf, HookFormat)>,
    pub plugins: Option<(PathBuf, PluginFormat)>,
    /// Why SkillPouch must only read this assistant, never write to it.
    pub read_only: Option<String>,
}

/// The assistants on this computer as places to link into. An assistant
/// that also reads a shared skill folder another one here fills gets no
/// skill links of its own, so no skill shows up twice.
pub fn targets(places: &Places) -> Vec<Target> {
    let mut filled: Vec<PathBuf> = Vec::new();
    layouts(places)
        .into_iter()
        .filter(|layout| layout.root.is_dir())
        .map(|layout| {
            let read_only = git_checkout(&layout.root).map(|repo| {
                format!(
                    "{} is part of the git checkout {}, so SkillPouch only reads it",
                    layout.root.display(),
                    repo.display()
                )
            });
            let shares = layout.also_reads.iter().any(|dir| filled.contains(dir));
            let skills = (!shares && read_only.is_none())
                .then(|| layout.skills.first().cloned())
                .flatten();
            if let Some(dir) = &skills {
                filled.push(dir.clone());
            }
            let (commands, format) = layout.commands;
            let mcp = layout
                .mcp
                .as_ref()
                .and_then(|(files, format)| Some((mcp_file(files)?, *format)));
            let plugins = layout.plugins.and_then(|(file, format)| {
                Some((
                    file.or_else(|| mcp.as_ref().map(|(path, _)| path.clone()))?,
                    format,
                ))
            });
            Target {
                id: layout.id,
                name: layout.name,
                instructions: layout.instructions.map(|file| layout.root.join(file)),
                skills,
                agents: layout
                    .agents
                    .first()
                    .map(|dir| (layout.root.join(dir), layout.agent_format)),
                commands: commands.first().map(|dir| (layout.root.join(dir), format)),
                mcp,
                hooks: layout.hooks,
                plugins,
                read_only,
                root: layout.root,
            }
        })
        .collect()
}

/// The git work tree `path` (links resolved) belongs to, if any. Writing
/// there would leave the checkout dirty and break its updates.
pub fn git_checkout(path: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    real.ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Every assistant whose folder exists, with what it holds.
pub fn discover(places: &Places) -> Vec<Harness> {
    let own = files::Own::new(&places.skillpouch);
    layouts(places)
        .into_iter()
        .filter(|layout| layout.root.is_dir())
        .map(|layout| scan(&layout, &own))
        .collect()
}

fn scan(layout: &Layout, own: &files::Own) -> Harness {
    // Keyed by kind and name: the first folder that has a name wins.
    let mut found: BTreeMap<(Kind, String), Item> = BTreeMap::new();
    let mut add = |item: Item| {
        found.entry((item.kind, item.name.clone())).or_insert(item);
    };
    if let Some(name) = layout.instructions {
        let path = layout.root.join(name);
        if let Some(bytes) = own.file(&path) {
            let file = path
                .file_name()
                .map_or(name.into(), |file| file.to_string_lossy().into_owned());
            add(Item {
                kind: Kind::Instructions,
                name: file,
                path,
                files: 1,
                bytes,
            });
        }
    }
    for folder in &layout.skills {
        files::skills(folder, own).into_iter().for_each(&mut add);
    }
    for folder in layout.agents {
        files::documents(&layout.root.join(folder), "md", false, Kind::Agent, own)
            .into_iter()
            .map(|mut item| {
                // Copilot also names them `<name>.agent.md`.
                if let Some(name) = item.name.strip_suffix(".agent") {
                    item.name = name.to_string();
                }
                item
            })
            .for_each(&mut add);
    }
    let (folders, format) = layout.commands;
    for folder in folders {
        files::documents(
            &layout.root.join(folder),
            format.extension(),
            true,
            Kind::Command,
            own,
        )
        .into_iter()
        .for_each(&mut add);
    }
    if let Some((files, format)) = &layout.mcp {
        let source = config::Source {
            files: files.clone(),
            format: format.file_format(),
            key: format.key(),
        };
        config::mcp_servers(&source).into_iter().for_each(&mut add);
    }
    Harness {
        id: layout.id,
        name: layout.name,
        root: layout.root.clone(),
        items: found.into_values().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn places(home: &Path) -> Places {
        Places::plain(home, &home.join(".skillpouch"))
    }

    fn write(path: &Path, text: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)
    }

    fn names(harness: &Harness, kind: Kind) -> Vec<&str> {
        harness
            .items
            .iter()
            .filter(|item| item.kind == kind)
            .map(|item| item.name.as_str())
            .collect()
    }

    #[test]
    fn finds_what_claude_code_holds() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let claude = home.path().join(".claude");
        write(&claude.join("CLAUDE.md"), "# Rules")?;
        write(
            &claude.join("skills/pdf/SKILL.md"),
            "---\nname: pdf-tools\ndescription: PDFs\n---\n# PDF",
        )?;
        write(&claude.join("skills/pdf/scripts/fill.py"), "print(1)")?;
        write(&claude.join("skills/pdf/node_modules/x/index.js"), "junk")?;
        write(&claude.join("skills/notes/README.md"), "not a skill")?;
        write(
            &claude.join("agents/reviewer.md"),
            "---\nname: reviewer\n---",
        )?;
        write(&claude.join("commands/git/commit.md"), "Commit")?;
        write(&claude.join("projects/x/session.jsonl"), "{}")?;
        write(
            &home.path().join(".claude.json"),
            r#"{"mcpServers":{"github":{"command":"gh"},"db":{"url":"x"}},"other":1}"#,
        )?;

        let found = discover(&places(home.path()));
        assert_eq!(found.len(), 1);
        let claude = &found[0];
        assert_eq!((claude.id, claude.name), ("claude-code", "Claude Code"));
        assert_eq!(names(claude, Kind::Instructions), ["CLAUDE.md"]);
        assert_eq!(names(claude, Kind::Skill), ["pdf-tools"]);
        assert_eq!(names(claude, Kind::Agent), ["reviewer"]);
        assert_eq!(names(claude, Kind::Command), ["git/commit"]);
        assert_eq!(names(claude, Kind::McpServer), ["db", "github"]);
        let skill = claude.items.iter().find(|i| i.kind == Kind::Skill);
        // SKILL.md and the script; node_modules is never counted.
        assert_eq!(skill.map(|s| s.files), Some(2));
        assert_eq!(
            claude.summary(),
            "CLAUDE.md, 1 skill, 1 agent, 1 command and 2 MCP servers"
        );
        Ok(())
    }

    #[test]
    fn reads_every_assistant_in_its_own_format() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let h = home.path();
        write(&h.join(".config/opencode/AGENTS.md"), "rules")?;
        write(&h.join(".config/opencode/skill/old/SKILL.md"), "# Old")?;
        write(&h.join(".config/opencode/agent/plan.md"), "plan")?;
        write(
            &h.join(".config/opencode/opencode.jsonc"),
            "{\n  // mine\n  \"mcp\": { \"fs\": { \"type\": \"local\", }, },\n  \"url\": \"http://x//y\"\n}",
        )?;
        write(&h.join(".codex/AGENTS.md"), "codex rules")?;
        write(&h.join(".codex/prompts/fix.md"), "fix it")?;
        write(
            &h.join(".codex/config.toml"),
            "model = \"gpt\"\n[mcp_servers.docs]\ncommand = \"npx\"\n",
        )?;
        write(&h.join(".gemini/GEMINI.md"), "gemini")?;
        write(&h.join(".gemini/commands/review.toml"), "prompt = \"x\"")?;
        write(&h.join(".gemini/settings.json"), "{ broken")?;

        let found = discover(&places(h));
        let ids: Vec<_> = found.iter().map(|h| h.id).collect();
        assert_eq!(ids, ["codex", "opencode", "gemini-cli"]);
        assert_eq!(names(&found[1], Kind::Skill), ["old"]);
        assert_eq!(names(&found[1], Kind::Agent), ["plan"]);
        assert_eq!(names(&found[1], Kind::McpServer), ["fs"]);
        assert_eq!(names(&found[0], Kind::Command), ["fix"]);
        assert_eq!(names(&found[0], Kind::McpServer), ["docs"]);
        assert_eq!(names(&found[2], Kind::Command), ["review"]);
        // A config that doesn't parse just has no servers.
        assert_eq!(names(&found[2], Kind::McpServer), Vec::<&str>::new());
        assert_eq!(found[2].summary(), "GEMINI.md and 1 command");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn skips_what_skillpouch_already_links() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let h = home.path();
        write(&h.join(".skillpouch/current/skills/a/SKILL.md"), "# A")?;
        write(&h.join("dotfiles/b/SKILL.md"), "# B")?;
        fs::create_dir_all(h.join(".claude/skills"))?;
        std::os::unix::fs::symlink(
            h.join(".skillpouch/current/skills/a"),
            h.join(".claude/skills/a"),
        )?;
        std::os::unix::fs::symlink(h.join("dotfiles/b"), h.join(".claude/skills/b"))?;
        let found = discover(&places(h));
        // Someone else's link is the user's own skill; ours is already synced.
        assert_eq!(names(&found[0], Kind::Skill), ["b"]);
        Ok(())
    }

    #[test]
    fn targets_skip_skill_folders_another_assistant_fills() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let h = home.path();
        for dir in [".claude", ".config/opencode", ".gemini"] {
            fs::create_dir_all(h.join(dir))?;
        }
        write(&h.join(".config/opencode/opencode.jsonc"), "{}")?;
        let found = targets(&places(h));
        let ids: Vec<_> = found.iter().map(|t| t.id).collect();
        assert_eq!(ids, ["claude-code", "opencode", "gemini-cli"]);
        assert_eq!(found[0].instructions, Some(h.join(".claude/CLAUDE.md")));
        assert_eq!(
            found[0].mcp,
            Some((h.join(".claude.json"), McpFormat::Claude))
        );
        // OpenCode reads ~/.claude/skills already.
        assert_eq!(found[1].skills, None);
        assert_eq!(
            found[1].agents,
            Some((h.join(".config/opencode/agents"), AgentFormat::OpenCode))
        );
        assert_eq!(
            found[1].mcp.as_ref().map(|(path, _)| path.clone()),
            Some(h.join(".config/opencode/opencode.jsonc"))
        );
        assert_eq!(
            found[2].commands,
            Some((h.join(".gemini/commands"), CommandFormat::Toml))
        );
        assert!(found.iter().all(|target| target.read_only.is_none()));
        Ok(())
    }

    #[test]
    fn folders_inside_a_git_checkout_are_only_read() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let h = home.path();
        fs::create_dir_all(h.join("dotfiles/.git"))?;
        fs::create_dir_all(h.join("dotfiles/opencode"))?;
        fs::create_dir_all(h.join(".config"))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(h.join("dotfiles/opencode"), h.join(".config/opencode"))?;
        #[cfg(not(unix))]
        fs::create_dir_all(h.join(".config/opencode/.git"))?;
        let found = targets(&places(h));
        assert_eq!(found.len(), 1);
        let reason = found[0].read_only.clone().unwrap_or_default();
        assert!(reason.contains("git checkout"), "{reason}");
        assert_eq!(found[0].skills, None);
        Ok(())
    }

    #[test]
    fn an_empty_assistant_says_so() {
        let harness = Harness {
            id: "codex",
            name: "Codex",
            root: PathBuf::new(),
            items: vec![],
        };
        assert_eq!(harness.summary(), "nothing yet");
    }
}
