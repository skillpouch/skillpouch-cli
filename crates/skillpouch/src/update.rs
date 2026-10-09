//! Keeping SkillPouch up to date on its own. Installed with npm, pnpm or
//! bun, it asks the npm registry for the latest version at most every few
//! hours and installs it with the same package manager: before a command
//! someone runs at the terminal, and in the background sync. Builds from
//! source and one-off `npx` runs are never touched.

use crate::context::{Context, VERSION};
use crate::ui;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::OnceLock;

pub const PACKAGE: &str = "skillpouch";
const REGISTRY: &str = "https://registry.npmjs.org";
/// How often the registry is asked.
const CHECK_EVERY: u64 = 6 * 60 * 60;
/// A version that failed to install is tried again after this long.
const RETRY_AFTER: u64 = 24 * 60 * 60;
/// Set on the command run again after an update, so it doesn't loop.
const UPDATED_ENV: &str = "SKILLPOUCH_UPDATED";

/// This program, as found when it started. Once the package manager has
/// replaced the file, Linux reports the running one as deleted.
pub fn exe() -> Option<PathBuf> {
    static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().ok()).clone()
}

/// Run from a package manager's throwaway cache (`npx`, `pnpm dlx`,
/// `bunx`): gone once the cache is cleared, so nothing may point at it.
pub fn temporary(exe: &Path) -> bool {
    exe.components().any(|part| {
        let part = part.as_os_str().to_string_lossy();
        part == "_npx" || part == "dlx" || part.starts_with("bunx-")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Npm,
    Pnpm,
    Bun,
}

impl Manager {
    fn program(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Bun => "bun",
        }
    }

    /// What someone types to install SkillPouch this way.
    pub fn install_line(self) -> String {
        match self {
            Self::Npm => format!("npm i -g {PACKAGE}"),
            Self::Pnpm => format!("pnpm add -g {PACKAGE}"),
            Self::Bun => format!("bun add -g {PACKAGE}"),
        }
    }
}

/// How this copy of SkillPouch was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    pub manager: Manager,
    /// Where the package manager (and Node) live, when it can be told; the
    /// background sync runs without the shell's `PATH`.
    pub bin_dir: Option<PathBuf>,
}

/// `None` for builds from source and temporary runs.
pub fn installer(exe: &Path) -> Option<Installer> {
    if temporary(exe) {
        return None;
    }
    let ancestor = |name: &str| exe.ancestors().find(|dir| dir.ends_with(name));
    let modules = exe
        .ancestors()
        .filter(|dir| dir.ends_with("node_modules"))
        .last()?;
    if ancestor(".pnpm").is_some() {
        // $PNPM_HOME/global/5/.pnpm/…; pnpm itself sits in $PNPM_HOME.
        let bin_dir = ancestor("global")
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        return Some(Installer {
            manager: Manager::Pnpm,
            bin_dir,
        });
    }
    if let Some(bun) = ancestor(".bun") {
        return Some(Installer {
            manager: Manager::Bun,
            bin_dir: Some(bun.join("bin")),
        });
    }
    // <prefix>/lib/node_modules on macOS and Linux, <prefix>\node_modules on Windows.
    let parent = modules.parent()?;
    let bin_dir = if parent.ends_with("lib") {
        parent.parent().map(|prefix| prefix.join("bin"))
    } else {
        Some(parent.to_path_buf())
    };
    Some(Installer {
        manager: Manager::Npm,
        bin_dir,
    })
}

impl Installer {
    fn command(&self, version: &str) -> Command {
        let program = self.manager.program();
        let found = self
            .bin_dir
            .as_ref()
            .map(|dir| dir.join(program))
            .filter(|path| path.exists());
        let mut command = Command::new(found.unwrap_or_else(|| PathBuf::from(program)));
        let verb = match self.manager {
            Manager::Npm => "install",
            Manager::Pnpm | Manager::Bun => "add",
        };
        command.args([verb, "-g", &format!("{PACKAGE}@{version}")]);
        if let Some(dir) = &self.bin_dir {
            // npm is a Node script: Node must be found next to it.
            let mut paths = vec![dir.clone()];
            paths.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            if let Ok(joined) = std::env::join_paths(paths) {
                command.env("PATH", joined);
            }
        }
        command
    }

    /// Installs `version`; the error is the package manager's last words.
    pub fn install(&self, version: &str) -> Result<(), String> {
        let output = self
            .command(version)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| format!("couldn't run {}: {error}", self.manager.program()))?;
        if output.status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&output.stderr);
        let last = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .find(|line| line.contains("EACCES") || line.contains("permission"))
            .or_else(|| text.lines().map(str::trim).rfind(|line| !line.is_empty()))
            .unwrap_or("it failed");
        Err(last.to_string())
    }
}

/// `latest` is a newer release than `current`. Pre-releases never are.
pub fn newer(latest: &str, current: &str) -> bool {
    let parse = |version: &str| -> Option<Vec<u64>> {
        if version.contains('-') {
            return None;
        }
        version
            .trim_start_matches('v')
            .split('.')
            .map(|part| part.parse().ok())
            .collect()
    };
    match (
        parse(latest),
        parse(current.split('-').next().unwrap_or(current)),
    ) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

/// `state/update.json`: when the registry was last asked, and what failed.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    checked_at: u64,
    latest: Option<String>,
    failed: Option<String>,
    failed_at: u64,
}

impl State {
    fn path(ctx: &Context) -> PathBuf {
        ctx.home.state_dir().join("update.json")
    }

    fn load(ctx: &Context) -> Self {
        std::fs::read(Self::path(ctx))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, ctx: &Context) {
        let _ = ctx.home.ensure_private_dir(&ctx.home.state_dir());
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = sp_core::home::write_private(&Self::path(ctx), &bytes);
        }
    }
}

/// Updating is on: not turned off in `config.toml` or the environment.
fn enabled(ctx: &Context) -> bool {
    let off = std::env::var("SKILLPOUCH_NO_UPDATE").is_ok_and(|value| !value.is_empty());
    !off && ctx.config.auto_update != Some(false)
}

/// The version to update to, if there is one worth trying now.
async fn pending(ctx: &Context, now: u64) -> Option<String> {
    let mut state = State::load(ctx);
    if now.saturating_sub(state.checked_at) >= CHECK_EVERY {
        let registry = std::env::var("SKILLPOUCH_NPM_REGISTRY").unwrap_or_else(|_| REGISTRY.into());
        let asked = ctx.client.npm_latest(&registry, PACKAGE).await;
        state.checked_at = now;
        if let Ok(latest) = asked {
            state.latest = Some(latest);
        }
        state.save(ctx);
    }
    let latest = state.latest.filter(|latest| newer(latest, VERSION))?;
    let failed_lately = state.failed.as_deref() == Some(latest.as_str())
        && now.saturating_sub(state.failed_at) < RETRY_AFTER;
    (!failed_lately).then_some(latest)
}

fn remember_failure(ctx: &Context, version: &str, now: u64) {
    let mut state = State::load(ctx);
    state.failed = Some(version.to_string());
    state.failed_at = now;
    state.save(ctx);
}

/// Before a command at the terminal: installs a newer release if there is
/// one, then runs the same command again with it. `Some` is that run's exit
/// code; `None` means carry on with this version.
pub async fn before_command(ctx: &Context) -> Option<ExitCode> {
    if !enabled(ctx) || std::env::var_os(UPDATED_ENV).is_some() {
        return None;
    }
    let exe = exe()?;
    let installer = installer(&exe)?;
    let now = crate::context::unix_now();
    let latest = pending(ctx, now).await?;
    if cfg!(windows) {
        // Windows won't let a running program be replaced.
        ui::hint(format!(
            "SkillPouch {latest} is out. Update with `{}`.",
            installer.manager.install_line()
        ));
        remember_failure(ctx, &latest, now);
        return None;
    }
    let line = installer.manager.install_line();
    let spinner = ui::Spinner::start(format!("Updating SkillPouch {VERSION} → {latest}…"));
    let version = latest.clone();
    let result = tokio::task::spawn_blocking(move || installer.install(&version))
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    spinner.clear();
    if let Err(error) = result {
        remember_failure(ctx, &latest, now);
        ui::warn(format!("Couldn't update to {latest}: {error}"));
        ui::hint(format!("Update by hand with `{line}`."));
        return None;
    }
    ui::success(format!("Updated to SkillPouch {latest}."));
    let status = Command::new(&exe)
        .args(std::env::args_os().skip(1))
        .env(UPDATED_ENV, "1")
        .status();
    Some(match status {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(_) => ExitCode::FAILURE,
    })
}

/// In the background sync: installs a newer release if there is one.
/// `true` once it is installed; the caller then starts over with it.
pub async fn in_background(ctx: &Context, log: &crate::daemon::Log) -> bool {
    if !enabled(ctx) || cfg!(windows) {
        return false;
    }
    let Some(installer) = exe().as_deref().and_then(installer) else {
        return false;
    };
    let now = crate::context::unix_now();
    let Some(latest) = pending(ctx, now).await else {
        return false;
    };
    let version = latest.clone();
    let result = tokio::task::spawn_blocking(move || installer.install(&version))
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    match result {
        Ok(()) => {
            log.line(&format!("updated to {latest}"));
            true
        }
        Err(error) => {
            remember_failure(ctx, &latest, now);
            log.line(&format!("couldn't update to {latest}: {error}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.1.2", "0.1.1"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.99.0"));
        assert!(!newer("0.1.1", "0.1.1"));
        assert!(!newer("0.1.0", "0.1.1"));
        assert!(!newer("0.2.0-beta.1", "0.1.1"));
        assert!(!newer("garbage", "0.1.1"));
    }

    #[test]
    fn tells_how_it_was_installed() {
        let npm = installer(Path::new(
            "/home/ada/.nvm/versions/node/v24.21.0/lib/node_modules/skillpouch/node_modules/@skillpouch/cli-linux-x64/bin/skillpouch",
        ));
        assert_eq!(
            npm,
            Some(Installer {
                manager: Manager::Npm,
                bin_dir: Some(PathBuf::from("/home/ada/.nvm/versions/node/v24.21.0/bin")),
            })
        );
        let pnpm = installer(Path::new(
            "/home/ada/.local/share/pnpm/global/5/.pnpm/@skillpouch+cli-linux-x64@0.1.1/node_modules/@skillpouch/cli-linux-x64/bin/skillpouch",
        ));
        assert_eq!(
            pnpm,
            Some(Installer {
                manager: Manager::Pnpm,
                bin_dir: Some(PathBuf::from("/home/ada/.local/share/pnpm")),
            })
        );
        let bun = installer(Path::new(
            "/home/ada/.bun/install/global/node_modules/@skillpouch/cli-linux-x64/bin/skillpouch",
        ));
        assert_eq!(
            bun.map(|i| (i.manager, i.bin_dir)),
            Some((Manager::Bun, Some(PathBuf::from("/home/ada/.bun/bin"))))
        );
        // Built from source, or a one-off npx run: left alone.
        assert_eq!(
            installer(Path::new("/src/skillpouch-cli/target/release/skillpouch")),
            None
        );
        let npx = Path::new(
            "/home/ada/.npm/_npx/1a2b/node_modules/@skillpouch/cli-linux-x64/bin/skillpouch",
        );
        assert!(temporary(npx));
        assert_eq!(installer(npx), None);
    }

    #[test]
    fn the_install_command_finds_node_next_to_npm() {
        let installer = Installer {
            manager: Manager::Npm,
            bin_dir: Some(PathBuf::from("/opt/node/bin")),
        };
        let command = installer.command("0.2.0");
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["install", "-g", "skillpouch@0.2.0"]);
        let path = command
            .get_envs()
            .find(|(name, _)| *name == "PATH")
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert!(path.starts_with("/opt/node/bin"), "{path}");
    }
}
