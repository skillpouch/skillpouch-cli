# AGENTS.md

Rules for anyone (human or AI agent) committing to this repository.

## Commit messages: Conventional Commits, always

Releases are fully automatic. After CI passes on `main`, semantic-release reads
the commit messages since the last release and decides the next version, the
changelog and whether to release at all. A message it can't read is a change
that never ships, or ships under the wrong version. CI (commitlint) and the
`commit-msg` hook reject anything else.

```
<type>(<scope>): <summary>

<body: what changed and why, wrapped prose>

<footer: BREAKING CHANGE: …, Refs: …>
```

- **type** decides the release:

  | type       | use for                                         | release |
  | ---------- | ----------------------------------------------- | ------- |
  | `feat`     | something users can do that they couldn't       | minor   |
  | `fix`      | a bug users could hit                           | patch   |
  | `perf`     | faster or lighter, same behaviour               | patch   |
  | `revert`   | undoing an earlier commit                       | patch   |
  | `refactor` | code change with no visible effect              | none    |
  | `test`     | tests only                                      | none    |
  | `docs`     | README, AGENTS.md, comments, other `.md` files  | none    |
  | `build`    | Cargo, toolchain, dependencies' configuration   | none    |
  | `ci`       | `.github/`, release scripts and config          | none    |
  | `chore`    | anything else that never reaches users          | none    |
  | `style`    | formatting only                                 | none    |

- **Breaking change** → major: add `!` after the type or scope
  (`feat(sync)!: …`) **and** a `BREAKING CHANGE: <what users must do>` footer.
  Breaking means users must act: a removed or renamed command or flag, a
  changed config or state file format, a server API this CLI no longer
  supports.
- **scope** is optional; when given it must be one of:
  `cli` (commands, bin crate), `tui`, `sync`, `harness`, `api`, `core`,
  `crypto`, `usage`, `npm` (launcher and packages), `release`, `ci`, `deps`.
- **summary**: imperative, lowercase start, no trailing period, at most 100
  characters for the whole first line. Say what changes for the user:
  `fix(sync): retry at once when the access token expired mid-sync`, not
  `fix: update sync.rs`.
- One logical change per commit. Don't mix a `feat` with unrelated `docs`;
  split them so the changelog stays true.
- Pick the type by what users get, not by the files touched: a bug fix that
  also edits the README is `fix`; a README-only change is `docs` and releases
  nothing, which is intended.

Examples:

```
feat(tui): show token usage per model on the Usage tab
fix(harness): find Codex skills in ~/.agents/skills
perf(sync): download item files in parallel
docs: explain how conflicts are resolved
ci(release): publish the GitHub release after npm
feat(cli)!: rename `skillpouch select` to `skillpouch choose`

BREAKING CHANGE: scripts calling `skillpouch select` must call `skillpouch choose`.
```

## Never by hand

- No version numbers: `Cargo.toml`, `Cargo.lock` and `npm/skillpouch/package.json`
  are stamped by the release.
- No `CHANGELOG.md` edits: it is generated from the commit messages.
- No `v*` tags and no GitHub releases: semantic-release creates them.
- No `chore(release): …` commits: only the release workflow writes those.
- No force-pushes to `main`: semantic-release needs the history it tagged.

## Before committing

Run once after cloning: `npm install` (release tooling and the `commit-msg`
hook). Then, for every change:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm test            # only when scripts/ changed
```

Every change comes with tests. Commit locally; pushing is up to the
maintainer.
