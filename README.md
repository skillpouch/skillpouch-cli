# skillpouch-cli

The `skillpouch` command-line tool and background sync daemon.

| Path | What |
|------|------|
| `crates/skillpouch/` | The `skillpouch` binary: commands, output, exit codes. |
| `crates/sp-core/` | `~/.skillpouch` (or `$SKILLPOUCH_HOME`), `config.toml`, credentials, canonical JSON, assistant detection. |
| `crates/sp-crypto/` | Device keys (P-256), DPoP proofs, PKCE, the login confirm code; the account roster, key envelopes and pouch names. |
| `crates/sp-api/` | HTTP client for the API: envelope, DPoP nonces, typed calls. |
| `crates/sp-harness/` | The AI assistants on this computer (Claude Code, Codex, OpenCode, Gemini CLI, Copilot CLI, Cursor, Windsurf, Qwen Code, Factory Droid, Crush): what they hold and where they read it. |
| `crates/sp-sync/` | The sync engine: the pouch as plain files in `~/.skillpouch/pouches/<id>/`, three-way compare with the server, encrypted commits, links into each assistant, and MCP servers, hooks and plugins in their configs. |
| `crates/sp-tui/` | The terminal UI (ratatui): overview with the animated mascot, pouches (pick the one this computer syncs), token usage, computers, account. |
| `crates/sp-usage/` | Token counts per day and model from Claude Code, Codex and OpenCode logs. Only counts leave the computer. |
| `npm/skillpouch/` | The npm package `skillpouch` (`npx skillpouch …`). Today a placeholder; later a thin launcher for the native binary. |

Server, web app and API contracts live in the separate `skillpouch` repository.

## Commands

```sh
skillpouch                  # interactive terminal: the TUI; pipes and scripts: a summary
skillpouch login [<connect-token>] [--name <name>] [--no-browser]
skillpouch status [--json]
skillpouch sync             # sync once, in the foreground
skillpouch daemon start     # keep syncing in the background (systemd / launchd user service)
skillpouch daemon stop | status
skillpouch logout
skillpouch --version
```

### How syncing works

- The active pouch lives as plain files in `~/.skillpouch/pouches/<id>/`, one folder
  per item (`skills/pdf/SKILL.md`, `agents/reviewer/AGENT.md`,
  `mcp-servers/github/server.json`). `~/.skillpouch/current` points at it.
- Each assistant gets links into `current/…`: `~/.claude/CLAUDE.md`,
  `~/.config/opencode/AGENTS.md` and the rest all point at the one set of
  instructions, so editing any of them edits the pouch. MCP servers are written
  into each assistant's config; only the entries SkillPouch wrote are ever changed.
- Hooks are stored once (`hooks/<name>/hook.json`, Claude Code event names) and
  written in each assistant's own format: Claude Code, Codex, Qwen Code and
  Factory (`hooks`), Gemini CLI (`BeforeTool`, `AfterTool`, … with timeouts in ms),
  Cursor (`~/.cursor/hooks.json`), Copilot CLI (its own file
  `~/.copilot/hooks/skillpouch.json`) and Windsurf (`pre_run_command`, …). An event
  an assistant doesn't have is left out there.
- Plugins: references (`plugin@marketplace`, or an npm package for OpenCode) and
  whole plugin folders, which Claude Code and Factory install from a `skillpouch`
  marketplace in the pouch's `plugins/` folder.
- Hooks and plugins from the pouch are installed on every computer as they arrive,
  like everything else. Only your own devices can write to the pouch (every change is
  signed and end-to-end encrypted).
- What you do in an assistant, with its own commands or by hand, is synced: a new
  skill, agent, command, MCP server, hook or plugin goes into the pouch; one you
  delete there (`claude mcp remove`, deleting a skill folder, turning a plugin off)
  is removed from the pouch and from every computer; a skill reinstalled over its
  link replaces the pouch's copy. Instructions are shared by every assistant, so a
  deleted `CLAUDE.md` is only linked again. If every SkillPouch item in a folder
  disappears at once, it's treated as a wipe and they are linked again.
- Anything a link replaces, and every item removed this way, is moved to
  `~/.skillpouch/backups/<time>/` first.
- An item changed on both sides keeps the pouch's version; this computer's is kept
  next to it as `<name>-conflict`. Nothing is lost.
- The daemon long-polls the server for changes and checks local folders every two
  seconds. Its log is `~/.skillpouch/logs/sync.log`; its state is in
  `~/.skillpouch/state/sync-status.json`, which the TUI shows on the overview.

Exit codes: `0` ok, `1` error, `2` usage, `3` not logged in, `130` cancelled.

## Development

The toolchain is pinned in `rust-toolchain.toml`; `rustup` installs it on first use.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Against the local dev server (`pnpm dev` in the `skillpouch` repo). The API
checks DPoP proofs against `API_PUBLIC_URL`, which is the Vite origin in dev:

```sh
SKILLPOUCH_HOME=/tmp/sp-dev SKILLPOUCH_API_URL=http://localhost:5173 cargo run -- login
```

## Publishing the npm package

```sh
cd npm/skillpouch
npm login
npm publish --access public
```

Never unpublish a version: publish a higher one instead.

## License

AGPL-3.0-only.
