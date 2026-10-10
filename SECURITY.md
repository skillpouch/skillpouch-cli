# Security policy

The SkillPouch CLI holds a computer's device keys and syncs end-to-end
encrypted pouches, so we take reports about its security seriously. Thank you
for helping keep it safe.

## Supported versions

| Component | Supported |
| --- | --- |
| CLI (this repository) | The latest release; fixes ship as a new release, not backports |
| Service, API and web app ([`skillpouch`][server]) | The `main` branch and the current hosted deployment |

The CLI updates itself by default, so most users get a fix with the next
release. Run `skillpouch --version` to check yours.

## Reporting a vulnerability

Please report privately, not in a public issue, pull request or Discord.

1. **Preferred:** GitHub private vulnerability reporting. Open this
   repository's **Security** tab and choose **Report a vulnerability**.
2. **Or email** [info@skillpouch.net](mailto:info@skillpouch.net) with
   "Security" in the subject. If you'd like to encrypt the details, say so in
   a first short message and we'll agree on a way.

If the issue is on the server side, you can report it in either repository.

### What to include

- the CLI version (`skillpouch --version`), operating system and how it was
  installed (npm, pnpm, bun, from source)
- what an attacker can do, and from which position (another user, a network
  observer, a malicious or compromised server, another local user or program)
- steps to reproduce or a proof of concept
- whether you'd like to be credited, and under which name

Please don't include other people's data. Use your own accounts.

### What happens next

This is a small project, so we can't promise fixed response times. We aim to:

- acknowledge your report within a few days
- tell you whether we can reproduce it and how serious we think it is
- keep you updated while we work on a fix, and agree on a disclosure date with
  you
- credit you in the advisory once the fix is out, unless you prefer not

We publish fixed vulnerabilities as GitHub security advisories.

## Scope

In scope:

- the end-to-end encryption in [`crates/sp-crypto`](crates/sp-crypto):
  device keys, DPoP proofs, PKCE login, HPKE welcome and key envelopes,
  roster verification, sealed objects
- credential and key storage ([`crates/sp-core`](crates/sp-core)), the
  background sync service, file linking and backups in assistant folders
- the API client ([`crates/sp-api`](crates/sp-api)) and self-update
- the server, API and web app, through [their repository][server]

Out of scope:

- attacks that need an already compromised operating system, administrator
  rights, or physical access to an unlocked computer
- reading `~/.skillpouch` as the same operating-system user (see below)
- denial of service through traffic volume
- reports from automated scanners without a demonstrated impact
- third-party services (GitHub, Google, npm) themselves

## Good faith

We won't pursue or support legal action against anyone who researches and
reports in good faith under this policy: you avoid privacy violations, data
destruction and service disruption, only access data that belongs to you,
stop and report as soon as you find a way to reach someone else's data, and
give us reasonable time to fix the issue before telling others. If you're
unsure whether something is allowed, ask us first.

## Security model in short

- **Content is encrypted before upload.** Pouch content and names are sealed
  on this computer; the service stores and relays ciphertext and never holds
  content keys. The formats are byte-compatible with the web app's
  [`@skillpouch/crypto`][crypto].
- **The account roster is verified locally.** The CLI replays the signed,
  append-only roster and only trusts keys and pouches it can verify
  ([`vault.rs`](crates/sp-crypto/src/vault.rs)), so the server can't add a
  device or swap keys unnoticed.
- **Device keys never leave the computer.** Logging in creates a signing and
  an encryption key pair here; a signed-in browser approves the device with an
  HPKE-sealed welcome, and pouch keys arrive in key envelopes sealed to this
  device. Requests prove possession of
  the signing key with DPoP ([`dpop.rs`](crates/sp-crypto/src/dpop.rs)).
- **Local storage is file-based.** Keys and tokens are kept in
  `~/.skillpouch/state/credentials.json`, mode `0600` inside a `0700` folder
  ([`credentials.rs`](crates/sp-core/src/credentials.rs)). Anything running as
  your user can read it; OS keystores are not used yet.
  `skillpouch logout` removes them.
- **Updates come from npm.** Self-update asks the npm registry for the latest
  version and installs it with your package manager; it does not download code
  from the SkillPouch server.

What the service can see: account and device identifiers, sizes and timing of
uploads, the number of pouches and files, IP addresses, and the token-usage
counts the CLI reports (counts only, never prompt text). The server-side model
is described in the [server repository's security policy][server-security].

[server]: https://github.com/skillpouch/skillpouch
[server-security]: https://github.com/skillpouch/skillpouch/blob/main/SECURITY.md
[crypto]: https://github.com/skillpouch/skillpouch/tree/main/packages/crypto
