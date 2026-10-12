//! Every assistant's global setup, as people write it, read the way a sync
//! reads it: what the setup screen lists, what goes into the pouch, and the
//! hooks and plugins brought in from the config files.

use serde_json::{Value, json};
use sp_harness::{Places, discover, targets};
use sp_sync::{adopt, hooks, plugins};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

type Res = Result<(), Box<dyn std::error::Error>>;

fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

fn skill(dir: &Path, name: &str) -> std::io::Result<()> {
    write(
        &dir.join(name).join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: Does {name}\n---\n# {name}\n"),
    )
}

const AGENT: &str = "---\nname: reviewer\ndescription: Reviews code\n---\nReview it.\n";
const COMMAND: &str = "---\ndescription: Ship it\n---\nShip $ARGUMENTS\n";
const TOML_COMMAND: &str = "description = \"Ship it\"\nprompt = \"Ship {{args}}\"\n";

/// A home with every assistant set up the way its documentation shows.
fn home(home: &Path, opencode_config: &str) -> std::io::Result<()> {
    let claude = home.join(".claude");
    write(&claude.join("CLAUDE.md"), "# Rules")?;
    skill(&claude.join("skills"), "pdf")?;
    write(&claude.join("agents/reviewer.md"), AGENT)?;
    write(&claude.join("commands/ship.md"), COMMAND)?;
    write(
        &home.join(".claude.json"),
        r#"{"numStartups": 3, "mcpServers": {
            "github": {"type": "stdio", "command": "npx", "args": ["-y", "gh-mcp"], "env": {"TOKEN": "t"}},
            "docs": {"type": "sse", "url": "https://docs.example/sse", "headers": {"A": "b"}}
        }}"#,
    )?;
    write(
        &claude.join("settings.json"),
        r#"{
            "hooks": {"PostToolUse": [{"matcher": "Edit|Write", "hooks": [
                {"type": "command", "command": "prettier --write .", "timeout": 30}
            ]}]},
            "enabledPlugins": {"review@acme": true, "off@acme": false},
            "extraKnownMarketplaces": {"acme": {"source": {"source": "github", "repo": "acme/plugins"}}}
        }"#,
    )?;

    let codex = home.join(".codex");
    write(&codex.join("AGENTS.md"), "# Rules")?;
    skill(&home.join(".agents/skills"), "shared")?;
    write(&codex.join("prompts/ship.md"), COMMAND)?;
    write(
        &codex.join("config.toml"),
        "model = \"gpt\"\n\n[mcp_servers.github]\ncommand = \"npx\"\nargs = [\"-y\", \"gh-mcp\"]\nenv = { TOKEN = \"t\" }\n\n[mcp_servers.docs]\nurl = \"https://docs.example/mcp\"\n",
    )?;
    write(
        &codex.join("hooks.json"),
        r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}"#,
    )?;

    let opencode = home.join(".config/opencode");
    write(&opencode.join("AGENTS.md"), "# Rules")?;
    skill(&opencode.join("skills"), "pdf")?;
    write(
        &opencode.join("agents/reviewer.md"),
        "---\ndescription: Reviews code\nmode: subagent\n---\nReview it.\n",
    )?;
    write(&opencode.join("commands/ship.md"), COMMAND)?;
    write(&opencode.join("opencode.jsonc"), opencode_config)?;

    let gemini = home.join(".gemini");
    write(&gemini.join("GEMINI.md"), "# Rules")?;
    skill(&gemini.join("skills"), "pdf")?;
    write(&gemini.join("agents/reviewer.md"), AGENT)?;
    write(&gemini.join("commands/ship.toml"), TOML_COMMAND)?;
    write(
        &gemini.join("settings.json"),
        r#"{"mcpServers": {
            "github": {"command": "npx", "args": ["-y", "gh-mcp"], "env": {"TOKEN": "t"}},
            "docs": {"httpUrl": "https://docs.example/mcp"},
            "events": {"url": "https://docs.example/sse"}
        },
        "hooks": {"AfterTool": [{"matcher": "write_file", "hooks": [
            {"name": "fmt", "type": "command", "command": "prettier --write .", "timeout": 30000}
        ]}]}}"#,
    )?;

    let antigravity = gemini.join("config");
    skill(&antigravity.join("skills"), "slides")?;
    write(&antigravity.join("agents/reviewer.md"), AGENT)?;
    write(
        &antigravity.join("mcp_config.json"),
        r#"{"mcpServers": {"docs": {"serverUrl": "https://docs.example/mcp"},
            "github": {"command": "npx", "args": ["-y", "gh-mcp"]}}}"#,
    )?;

    let copilot = home.join(".copilot");
    write(&copilot.join("copilot-instructions.md"), "# Rules")?;
    skill(&copilot.join("skills"), "pdf")?;
    write(&copilot.join("agents/reviewer.agent.md"), AGENT)?;
    write(
        &copilot.join("mcp-config.json"),
        r#"{"mcpServers": {
            "github": {"type": "local", "command": "npx", "args": ["-y", "gh-mcp"], "tools": ["*"]},
            "docs": {"type": "http", "url": "https://docs.example/mcp", "tools": ["*"]}
        }}"#,
    )?;
    write(
        &copilot.join("hooks/skillpouch.json"),
        r#"{"version": 1, "hooks": {"PostToolUse": [
            {"type": "command", "command": "prettier --write .", "matcher": "Edit", "timeoutSec": 30}
        ]}}"#,
    )?;

    let cursor = home.join(".cursor");
    skill(&cursor.join("skills"), "pdf")?;
    write(&cursor.join("commands/ship.md"), "Ship it\n")?;
    write(
        &cursor.join("mcp.json"),
        r#"{"mcpServers": {"github": {"command": "npx", "args": ["-y", "gh-mcp"]},
            "docs": {"url": "https://docs.example/mcp"}}}"#,
    )?;
    write(
        &cursor.join("hooks.json"),
        r#"{"version": 1, "hooks": {"afterFileEdit": [{"command": "ignored"}],
            "postToolUse": [{"command": "prettier --write .", "matcher": "Edit"}]}}"#,
    )?;

    let windsurf = home.join(".codeium/windsurf");
    write(&windsurf.join("memories/global_rules.md"), "# Rules")?;
    write(
        &windsurf.join("global_workflows/ship.md"),
        "---\ndescription: Ship it\n---\nShip\n",
    )?;
    write(
        &windsurf.join("mcp_config.json"),
        r#"{"mcpServers": {"docs": {"serverUrl": "https://docs.example/mcp"},
            "github": {"command": "npx", "args": ["-y", "gh-mcp"]}}}"#,
    )?;
    write(
        &windsurf.join("hooks.json"),
        r#"{"hooks": {"post_write_code": [{"command": "prettier --write ."}]}}"#,
    )?;

    let qwen = home.join(".qwen");
    write(&qwen.join("QWEN.md"), "# Rules")?;
    write(&qwen.join("agents/reviewer.md"), AGENT)?;
    write(&qwen.join("commands/ship.toml"), TOML_COMMAND)?;
    write(
        &qwen.join("settings.json"),
        r#"{"mcpServers": {"github": {"command": "npx", "args": ["-y", "gh-mcp"]},
            "docs": {"httpUrl": "https://docs.example/mcp"}},
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}"#,
    )?;

    let factory = home.join(".factory");
    skill(&factory.join("skills"), "pdf")?;
    write(
        &factory.join("droids/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews code\nmodel: inherit\n---\nReview it.\n",
    )?;
    write(&factory.join("commands/ship.md"), COMMAND)?;
    write(
        &factory.join("mcp.json"),
        r#"{"mcpServers": {"github": {"type": "stdio", "command": "npx", "args": ["-y", "gh-mcp"]},
            "docs": {"type": "http", "url": "https://docs.example/mcp"}}}"#,
    )?;
    write(
        &factory.join("settings.json"),
        r#"{"model": "x", "enabledPlugins": {"lint@acme": true},
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}"#,
    )?;

    let crush = home.join(".config/crush");
    write(&crush.join("CRUSH.md"), "# Rules")?;
    skill(&crush.join("skills"), "pdf")?;
    Ok(())
}

const OPENCODE_2: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  // mine
  "mcp": {
    "servers": {
      "github": {
        "type": "local",
        "command": ["npx", "-y", "gh-mcp"],
        "environment": {"TOKEN": "t"},
      },
      "docs": {"type": "remote", "url": "https://docs.example/mcp", "oauth": false},
      "off": {"type": "local", "command": ["off-mcp"], "disabled": true},
    },
  },
  "plugins": [
    "opencode-wakatime",
    "@me/tool@1.2",
    {"package": "opencode-notify", "options": {"sound": true}},
  ],
}"#;

const OPENCODE_1: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "github": {"type": "local", "command": ["npx", "-y", "gh-mcp"], "environment": {"TOKEN": "t"}, "enabled": true},
    "docs": {"type": "remote", "url": "https://docs.example/mcp"},
  },
  "plugin": ["opencode-wakatime", ["opencode-notify", {"sound": true}]],
}"#;

fn places(home: &Path) -> Places {
    Places::plain(home, &home.join(".skillpouch"))
}

/// What a sync brings into the pouch from each assistant: `kind/name`.
fn found(home: &Path) -> BTreeMap<&'static str, Vec<String>> {
    discover(&places(home))
        .iter()
        .map(|harness| {
            let mut items: Vec<String> = adopt::found_in(harness)
                .iter()
                .map(|found| format!("{}/{}", found.item.kind, found.item.name))
                .collect();
            items.sort();
            (harness.id, items)
        })
        .collect()
}

fn server(home: &Path, id: &str, name: &str) -> Value {
    discover(&places(home))
        .iter()
        .filter(|harness| harness.id == id)
        .flat_map(adopt::found_in)
        .find(|found| found.item.kind == "mcp_server" && found.item.name == name)
        .and_then(|found| serde_json::from_slice(&found.item.files["server.json"].bytes).ok())
        .unwrap_or_default()
}

fn items(list: &[&str]) -> Vec<String> {
    list.iter().map(|item| item.to_string()).collect()
}

#[test]
fn every_assistant_is_read_whole() -> Res {
    let dir = tempfile::tempdir()?;
    home(dir.path(), OPENCODE_2)?;
    let found = found(dir.path());
    let everything = |extra: &[&str]| {
        let mut all = items(&[
            "agent/reviewer",
            "command/ship",
            "instructions/instructions",
            "mcp_server/docs",
            "mcp_server/github",
            "skill/pdf",
        ]);
        all.extend(items(extra));
        all.sort();
        all
    };
    assert_eq!(found["claude-code"], everything(&[]));
    assert_eq!(
        found["codex"],
        items(&[
            "command/ship",
            "instructions/instructions",
            "mcp_server/docs",
            "mcp_server/github",
            "skill/shared",
        ])
    );
    // The disabled server is still the user's: it comes along.
    assert_eq!(found["opencode"], everything(&["mcp_server/off"]));
    assert_eq!(found["gemini-cli"], everything(&["mcp_server/events"]));
    assert_eq!(
        found["antigravity"],
        items(&[
            "agent/reviewer",
            "instructions/instructions",
            "mcp_server/docs",
            "mcp_server/github",
            "skill/slides",
        ])
    );
    assert_eq!(
        found["copilot"],
        items(&[
            "agent/reviewer",
            "instructions/instructions",
            "mcp_server/docs",
            "mcp_server/github",
            "skill/pdf",
        ])
    );
    assert_eq!(
        found["cursor"],
        items(&[
            "command/ship",
            "mcp_server/docs",
            "mcp_server/github",
            "skill/pdf"
        ])
    );
    assert_eq!(
        found["windsurf"],
        items(&[
            "command/ship",
            "instructions/instructions",
            "mcp_server/docs",
            "mcp_server/github",
        ])
    );
    assert_eq!(
        found["qwen-code"],
        items(&[
            "agent/reviewer",
            "command/ship",
            "instructions/instructions",
            "mcp_server/docs",
            "mcp_server/github",
        ])
    );
    assert_eq!(
        found["factory"],
        items(&[
            "agent/reviewer",
            "command/ship",
            "mcp_server/docs",
            "mcp_server/github",
            "skill/pdf",
        ])
    );
    assert_eq!(
        found["crush"],
        items(&["instructions/instructions", "skill/pdf"])
    );
    Ok(())
}

#[test]
fn every_mcp_dialect_becomes_the_same_server() -> Res {
    let dir = tempfile::tempdir()?;
    home(dir.path(), OPENCODE_2)?;
    let home = dir.path();
    let gh = |env: Value| json!({ "name": "github", "description": "", "command": "npx", "args": ["-y", "gh-mcp"], "env": env });
    let docs = |kind: &str, url: &str| json!({ "name": "docs", "description": "", "type": kind, "url": url, "headers": {} });
    let token = json!({ "TOKEN": "t" });
    for id in ["claude-code", "codex", "opencode", "gemini-cli"] {
        assert_eq!(server(home, id, "github"), gh(token.clone()), "{id}");
    }
    for id in [
        "antigravity",
        "copilot",
        "cursor",
        "windsurf",
        "qwen-code",
        "factory",
    ] {
        assert_eq!(server(home, id, "github"), gh(json!({})), "{id}");
    }
    let mut sse = docs("sse", "https://docs.example/sse");
    sse["headers"] = json!({ "A": "b" });
    assert_eq!(server(home, "claude-code", "docs"), sse);
    let mut events = docs("sse", "https://docs.example/sse");
    events["name"] = json!("events");
    assert_eq!(server(home, "gemini-cli", "events"), events);
    for id in [
        "codex",
        "opencode",
        "gemini-cli",
        "antigravity",
        "copilot",
        "cursor",
        "windsurf",
        "qwen-code",
        "factory",
    ] {
        assert_eq!(
            server(home, id, "docs"),
            docs("http", "https://docs.example/mcp"),
            "{id}"
        );
    }
    Ok(())
}

#[test]
fn opencode_1_configs_still_read() -> Res {
    let dir = tempfile::tempdir()?;
    home(dir.path(), OPENCODE_1)?;
    assert_eq!(
        found(dir.path())["opencode"],
        items(&[
            "agent/reviewer",
            "command/ship",
            "instructions/instructions",
            "mcp_server/docs",
            "mcp_server/github",
            "skill/pdf",
        ])
    );
    assert_eq!(
        server(dir.path(), "opencode", "github")["env"],
        json!({ "TOKEN": "t" })
    );
    let target = targets(&places(dir.path()))
        .into_iter()
        .find(|target| target.id == "opencode")
        .ok_or("no opencode")?;
    let (path, format) = target.plugins.ok_or("no plugins")?;
    let npm: Vec<_> = plugins::added(&path, format, None, &BTreeMap::new())
        .into_iter()
        .filter_map(|plugin| plugin.npm)
        .collect();
    assert_eq!(npm, ["opencode-notify", "opencode-wakatime"]);
    Ok(())
}

#[test]
fn every_hooks_dialect_is_brought_in() -> Res {
    let dir = tempfile::tempdir()?;
    home(dir.path(), OPENCODE_2)?;
    let mut found: BTreeMap<&str, Vec<(String, String, String)>> = BTreeMap::new();
    for target in targets(&places(dir.path())) {
        let Some((path, format)) = &target.hooks else {
            continue;
        };
        let added = hooks::changes(path, *format, None).added;
        found.insert(
            target.id,
            added
                .into_iter()
                .map(|added| (added.hook.event, added.hook.matcher, added.hook.command))
                .collect(),
        );
    }
    let hook = |event: &str, matcher: &str, command: &str| {
        vec![(event.into(), matcher.into(), command.into())]
    };
    let format = hook("PostToolUse", "Edit|Write", "prettier --write .");
    let done = hook("Stop", "", "say done");
    assert_eq!(found["claude-code"], format);
    assert_eq!(found["codex"], done);
    assert_eq!(
        found["gemini-cli"],
        hook("PostToolUse", "write_file", "prettier --write .")
    );
    assert_eq!(
        found["copilot"],
        hook("PostToolUse", "Edit", "prettier --write .")
    );
    // `afterFileEdit` has no Claude Code event: it stays Cursor's.
    assert_eq!(
        found["cursor"],
        hook("PostToolUse", "Edit", "prettier --write .")
    );
    assert_eq!(found["windsurf"], format);
    assert_eq!(found["qwen-code"], done);
    assert_eq!(found["factory"], done);
    // OpenCode, Antigravity and Crush have no hooks file.
    let ids: BTreeSet<_> = found.keys().copied().collect();
    assert_eq!(
        ids,
        BTreeSet::from([
            "claude-code",
            "codex",
            "copilot",
            "cursor",
            "factory",
            "gemini-cli",
            "qwen-code",
            "windsurf",
        ])
    );
    Ok(())
}

#[test]
fn every_plugin_list_is_brought_in() -> Res {
    let dir = tempfile::tempdir()?;
    home(dir.path(), OPENCODE_2)?;
    let mut found: BTreeMap<&str, Vec<plugins::Plugin>> = BTreeMap::new();
    for target in targets(&places(dir.path())) {
        if let Some((path, format)) = &target.plugins {
            found.insert(
                target.id,
                plugins::added(path, *format, None, &BTreeMap::new()),
            );
        }
    }
    assert_eq!(found.len(), 3, "{found:?}");
    let claude = &found["claude-code"];
    assert_eq!(claude.len(), 1);
    assert_eq!(claude[0].source.as_deref(), Some("review@acme"));
    assert_eq!(
        claude[0].marketplace,
        Some(json!({"source": "github", "repo": "acme/plugins"}))
    );
    let factory: Vec<_> = found["factory"].iter().map(|p| p.source.clone()).collect();
    assert_eq!(factory, [Some("lint@acme".to_string())]);
    let opencode: Vec<_> = found["opencode"]
        .iter()
        .map(|p| (p.name.as_str(), p.npm.as_deref().unwrap_or_default()))
        .collect();
    assert_eq!(
        opencode,
        [
            ("tool", "@me/tool@1.2"),
            ("opencode-notify", "opencode-notify"),
            ("opencode-wakatime", "opencode-wakatime"),
        ]
    );
    Ok(())
}
