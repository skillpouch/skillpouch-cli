# skillpouch-cli

The `skillpouch` command-line tool and background sync daemon.

| Path | What |
|------|------|
| `npm/skillpouch/` | The npm package `skillpouch` (`npx skillpouch …`). Today a placeholder; later a thin launcher for the native binary. |
| `crates/` | *(coming)* The Rust workspace: core, harness adapters, store, API client, sync engine, daemon, TUI. |

Server, web app and API contracts live in the separate `skillpouch` repository.

## Publishing the npm package

```sh
cd npm/skillpouch
npm login
npm publish --access public
```

Never unpublish a version: publish a higher one instead.

## License

AGPL-3.0-only.
