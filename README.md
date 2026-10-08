# skillpouch-cli

The `skillpouch` command-line tool and background sync daemon.

| Path | What |
|------|------|
| `crates/skillpouch/` | The `skillpouch` binary: commands, output, exit codes. |
| `crates/sp-core/` | `~/.skillpouch` (or `$SKILLPOUCH_HOME`), `config.toml`, credentials, canonical JSON, assistant detection. |
| `crates/sp-crypto/` | Device keys (P-256), DPoP proofs, PKCE, the login confirm code. |
| `crates/sp-api/` | HTTP client for the API: envelope, DPoP nonces, typed calls. |
| `crates/sp-tui/` | The terminal UI (ratatui) and the logo: a real picture where the terminal supports one (sixel, kitty, iTerm2), smoothed half-blocks elsewhere. |
| `npm/skillpouch/` | The npm package `skillpouch` (`npx skillpouch …`). Today a placeholder; later a thin launcher for the native binary. |

Server, web app and API contracts live in the separate `skillpouch` repository.

## Commands

```sh
skillpouch                  # interactive terminal: the TUI; pipes and scripts: a summary
skillpouch login [<connect-token>] [--name <name>] [--no-browser]
skillpouch status [--json]
skillpouch logout
skillpouch --version
```

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
