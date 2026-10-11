//! Keeping SkillPouch up to date on its own. Installed with npm, pnpm or
//! bun, it asks the npm registry for the latest version and installs it
//! with the same package manager: before a command someone runs at the
//! terminal (at most every few hours), in the background sync (hourly), and
//! behind the dashboard each time it opens. Builds from source and one-off
//! `npx` runs are never touched.

use crate::context::{Context, VERSION};
use crate::ui;
use serde::{Deserialize, Serialize};
use sp_tui::Upgrade;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::OnceLock;

pub const PACKAGE: &str = "skillpouch";
const REGISTRY: &str = "https://registry.npmjs.org";
/// How often a command at the terminal asks the registry, so commands stay
/// quick.
const COMMAND_EVERY: u64 = 6 * 60 * 60;
/// How often the background sync asks.
const BACKGROUND_EVERY: u64 = 60 * 60;
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

    /// The files the package manager can be, most likely first. On Windows
    /// npm is `npm.cmd`, which isn't found under its bare name, and the
    /// extensionless `npm` next to it is a shell script Windows can't run.
    fn files(self, windows: bool) -> &'static [&'static str] {
        match (self, windows) {
            (Self::Npm, true) => &["npm.cmd", "npm.exe"],
            (Self::Pnpm, true) => &["pnpm.exe", "pnpm.cmd"],
            (Self::Bun, true) => &["bun.exe", "bun.cmd"],
            (Self::Npm, false) => &["npm"],
            (Self::Pnpm, false) => &["pnpm"],
            (Self::Bun, false) => &["bun"],
        }
    }

    /// The package manager in the first of `dirs` that has it.
    fn find(self, windows: bool, dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
        dirs.into_iter().find_map(|dir| {
            self.files(windows)
                .iter()
                .map(|file| dir.join(file))
                .find(|path| path.is_file())
        })
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
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs = self
            .bin_dir
            .iter()
            .cloned()
            .chain(std::env::split_paths(&path));
        let found = self.manager.find(cfg!(windows), dirs);
        let mut command = Command::new(
            found.unwrap_or_else(|| PathBuf::from(self.manager.files(cfg!(windows))[0])),
        );
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

/// The npm package this program came in: `…/node_modules/skillpouch`.
fn package_dir(exe: &Path) -> Option<&Path> {
    exe.ancestors()
        .filter(|dir| {
            dir.file_name() == Some(std::ffi::OsStr::new(PACKAGE))
                && dir
                    .parent()
                    .is_some_and(|parent| parent.ends_with("node_modules"))
        })
        .last()
}

/// npm moves the package it replaces aside as `.skillpouch-<hash>` (up to
/// eight letters and digits) and deletes it once installed. On Windows a
/// program still running from it keeps it there.
fn leftover(name: &str) -> bool {
    name.strip_prefix(&format!(".{PACKAGE}-"))
        .is_some_and(|hash| {
            (1..=8).contains(&hash.len()) && hash.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

/// Deletes what earlier updates left next to the installed package, once
/// nothing runs from it any more. Quiet, and never fails anything.
pub fn tidy() {
    if !cfg!(windows) {
        return;
    }
    let Some(exe) = exe() else {
        return;
    };
    if installer(&exe).is_some_and(|installer| installer.manager == Manager::Npm) {
        tidy_next_to(&exe);
    }
}

fn tidy_next_to(exe: &Path) {
    let Some(modules) = package_dir(exe).and_then(Path::parent) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(modules) else {
        return;
    };
    for entry in entries.flatten() {
        // Folders only, never what a link points to.
        let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if folder && entry.file_name().to_str().is_some_and(leftover) {
            // Still in use: whatever is left goes next time.
            let _ = std::fs::remove_dir_all(entry.path());
        }
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

/// Who looks for a new release, which decides how often the registry is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asker {
    Command,
    Background,
    /// The dashboard asks every time it opens: it doesn't wait for the answer.
    Dashboard,
}

impl Asker {
    fn every(self) -> u64 {
        match self {
            Self::Command => COMMAND_EVERY,
            Self::Background => BACKGROUND_EVERY,
            Self::Dashboard => 0,
        }
    }

    /// The registry was last asked at `checked_at`: time to ask again.
    fn due(self, checked_at: u64, now: u64) -> bool {
        now.saturating_sub(checked_at) >= self.every()
    }
}

/// A newer release than this one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// Worth installing now.
    Install(String),
    /// Installing it failed lately; it is tried again later.
    FailedLately(String),
}

/// The newer release, if there is one.
async fn pending(ctx: &Context, now: u64, asker: Asker) -> Option<Pending> {
    let mut state = State::load(ctx);
    if asker.due(state.checked_at, now) {
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
    Some(if failed_lately {
        Pending::FailedLately(latest)
    } else {
        Pending::Install(latest)
    })
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
    let Pending::Install(latest) = pending(ctx, now, Asker::Command).await? else {
        return None;
    };
    let line = installer.manager.install_line();
    let spinner = ui::Spinner::start(format!("Updating SkillPouch {VERSION} → {latest}…"));
    let result = install(ctx, installer, &latest, LOCKED_WHILE_RUNNING).await;
    spinner.clear();
    if let Err(error) = result {
        remember_failure(ctx, &latest, now);
        ui::warn(format!("Couldn't update to {latest}: {error}"));
        if LOCKED_WHILE_RUNNING && in_use(&error) {
            ui::hint(format!(
                "SkillPouch was in use. Update by hand with `{line}` once this command is done."
            ));
        } else {
            ui::hint(format!("Update by hand with `{line}`."));
        }
        return None;
    }
    ui::success(format!("Updated to SkillPouch {latest}."));
    Some(run_again())
}

/// Runs the same command again with the program now installed, and gives
/// back its exit code.
pub fn run_again() -> ExitCode {
    let Some(exe) = exe() else {
        return ExitCode::FAILURE;
    };
    let status = Command::new(&exe)
        .args(std::env::args_os().skip(1))
        .env(UPDATED_ENV, "1")
        .status();
    match status {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(_) => ExitCode::FAILURE,
    }
}

/// Windows won't let a running program be replaced.
const LOCKED_WHILE_RUNNING: bool = cfg!(windows);

/// What installing a new version takes, apart so the order can be tested.
trait Steps {
    fn stop_sync(&mut self);
    fn install(&mut self, version: &str) -> Result<(), String>;
    fn start_sync(&mut self);
}

/// Installs `version`. With `free_program` the background sync lets go of
/// the program first, and starts again afterwards on whichever version is
/// then installed, whether the install worked or not.
fn install_with(steps: &mut impl Steps, version: &str, free_program: bool) -> Result<(), String> {
    if free_program {
        steps.stop_sync();
    }
    let result = steps.install(version);
    if free_program {
        steps.start_sync();
    }
    result
}

struct OnThisComputer<'a> {
    ctx: &'a Context,
    installer: Installer,
}

impl Steps for OnThisComputer<'_> {
    fn stop_sync(&mut self) {
        let _ = crate::daemon::stop(self.ctx);
    }

    fn install(&mut self, version: &str) -> Result<(), String> {
        self.installer.install(version)
    }

    fn start_sync(&mut self) {
        crate::daemon::ensure_running(self.ctx);
    }
}

async fn install(
    ctx: &Context,
    installer: Installer,
    version: &str,
    free_program: bool,
) -> Result<(), String> {
    tokio::task::block_in_place(|| {
        install_with(
            &mut OnThisComputer { ctx, installer },
            version,
            free_program,
        )
    })
}

/// The package manager couldn't replace a program in use (Windows).
fn in_use(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "ebusy",
        "eperm",
        "resource busy",
        "being used by another process",
    ]
    .iter()
    .any(|sign| error.contains(sign))
}

/// Behind the dashboard, which opens without waiting: installs a newer
/// release if there is one, telling the dashboard how it goes. It offers to
/// restart once the new version is in. Silent when updating is off or this
/// copy can't be updated.
pub async fn in_dashboard(ctx: &Context, tell: impl Fn(Upgrade)) {
    if !enabled(ctx) || std::env::var_os(UPDATED_ENV).is_some() {
        return;
    }
    let Some(installer) = exe().as_deref().and_then(installer) else {
        return;
    };
    tell(Upgrade::Checking);
    let now = crate::context::unix_now();
    let line = installer.manager.install_line();
    let latest = match pending(ctx, now, Asker::Dashboard).await {
        None => return tell(Upgrade::UpToDate),
        // Said every time, until it is installed one way or another.
        Some(Pending::FailedLately(latest)) => {
            return tell(Upgrade::Failed {
                reason: format!("SkillPouch {latest} is out, but installing it failed"),
                hint: Some(format!("Run `{line}`.")),
            });
        }
        Some(Pending::Install(latest)) => latest,
    };
    tell(Upgrade::Installing(latest.clone()));
    match install(ctx, installer, &latest, LOCKED_WHILE_RUNNING).await {
        Ok(()) => tell(Upgrade::Installed(latest)),
        Err(error) => {
            remember_failure(ctx, &latest, now);
            // On Windows this very program can still be in the way.
            tell(if LOCKED_WHILE_RUNNING && in_use(&error) {
                Upgrade::Failed {
                    reason: "SkillPouch is in use".into(),
                    hint: Some(format!("Quit, then run `{line}`.")),
                }
            } else {
                Upgrade::Failed {
                    reason: error,
                    hint: None,
                }
            });
        }
    }
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
    let Some(Pending::Install(latest)) = pending(ctx, now, Asker::Background).await else {
        return false;
    };
    match install(ctx, installer, &latest, false).await {
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
    fn the_dashboard_asks_every_time_and_commands_every_six_hours() {
        let checked = 1_000_000;
        let hour = 60 * 60;
        // Opening the dashboard asks the registry, however recently it was asked.
        assert!(Asker::Dashboard.due(checked, checked));
        // The background sync asks hourly.
        assert_eq!(Asker::Background.every(), hour);
        assert!(!Asker::Background.due(checked, checked + hour - 1));
        assert!(Asker::Background.due(checked, checked + hour));
        // Commands at the terminal stay quick: every six hours.
        assert!(!Asker::Command.due(checked, checked + 5 * hour));
        assert!(Asker::Command.due(checked, checked + 6 * hour));
    }

    /// Writes down each step instead of taking it.
    struct Recorded {
        steps: Vec<String>,
        fails: bool,
    }

    impl Steps for Recorded {
        fn stop_sync(&mut self) {
            self.steps.push("stop sync".into());
        }

        fn install(&mut self, version: &str) -> Result<(), String> {
            self.steps.push(format!("install {version}"));
            if self.fails {
                Err("npm error code EBUSY".into())
            } else {
                Ok(())
            }
        }

        fn start_sync(&mut self) {
            self.steps.push("start sync".into());
        }
    }

    #[test]
    fn on_windows_the_sync_lets_go_of_the_program_while_it_installs() {
        let mut recorded = Recorded {
            steps: Vec::new(),
            fails: false,
        };
        assert_eq!(install_with(&mut recorded, "0.3.0", true), Ok(()));
        assert_eq!(recorded.steps, ["stop sync", "install 0.3.0", "start sync"]);
        // Failed or not, it syncs again afterwards.
        let mut recorded = Recorded {
            steps: Vec::new(),
            fails: true,
        };
        assert!(install_with(&mut recorded, "0.3.0", true).is_err());
        assert_eq!(recorded.steps, ["stop sync", "install 0.3.0", "start sync"]);
        // Elsewhere (and in the background sync itself) it just installs.
        let mut recorded = Recorded {
            steps: Vec::new(),
            fails: false,
        };
        assert_eq!(install_with(&mut recorded, "0.3.0", false), Ok(()));
        assert_eq!(recorded.steps, ["install 0.3.0"]);
    }

    #[test]
    fn tells_a_program_in_use_from_other_failures() {
        assert!(in_use(
            "npm error code EBUSY: resource busy or locked, rename 'C:\\…\\skillpouch.exe'"
        ));
        assert!(in_use("npm error code EPERM"));
        assert!(in_use(
            "The process cannot access the file because it is being used by another process."
        ));
        assert!(!in_use("npm error code E404"));
        assert!(!in_use("npm error code EACCES: permission denied"));
    }

    #[test]
    fn knows_what_npm_leaves_behind() {
        assert!(leftover(".skillpouch-Ab3dE9xZ"));
        assert!(leftover(".skillpouch-a1"));
        // Not npm's: too long, odd characters, other packages, the real one.
        assert!(!leftover(".skillpouch-Ab3dE9xZq"));
        assert!(!leftover(".skillpouch-ab_cd"));
        assert!(!leftover(".skillpouch-"));
        assert!(!leftover(".skillpouch"));
        assert!(!leftover("skillpouch"));
        assert!(!leftover(".skillpouchx-Ab3dE9xZ"));
        assert!(!leftover(".typescript-Ab3dE9xZ"));
    }

    #[test]
    fn finds_the_package_the_program_came_in() {
        let exe = Path::new(
            "/Users/ada/AppData/Roaming/npm/node_modules/skillpouch/node_modules/@skillpouch/cli-win32-x64/bin/skillpouch.exe",
        );
        assert_eq!(
            package_dir(exe),
            Some(Path::new(
                "/Users/ada/AppData/Roaming/npm/node_modules/skillpouch"
            ))
        );
        assert_eq!(
            package_dir(Path::new("/src/skillpouch/target/release/skillpouch")),
            None
        );
    }

    #[test]
    fn deletes_only_npm_leftovers_next_to_the_package() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let modules = root.path().join("node_modules");
        let bin = modules.join("skillpouch/node_modules/@skillpouch/cli-win32-x64/bin");
        std::fs::create_dir_all(&bin)?;
        let exe = bin.join("skillpouch.exe");
        std::fs::write(&exe, b"")?;
        let old = modules.join(".skillpouch-Ab3dE9xZ/node_modules/@skillpouch/cli-win32-x64/bin");
        std::fs::create_dir_all(&old)?;
        std::fs::write(old.join("skillpouch.exe"), b"")?;
        for kept in [".skillpouch-Ab3dE9xZq", ".typescript-Ab3dE9xZ", ".bin"] {
            std::fs::create_dir_all(modules.join(kept))?;
        }
        std::fs::write(modules.join(".skillpouch-file0001"), b"")?;

        tidy_next_to(&exe);

        assert!(!modules.join(".skillpouch-Ab3dE9xZ").exists());
        assert!(exe.exists());
        for kept in [
            ".skillpouch-Ab3dE9xZq",
            ".typescript-Ab3dE9xZ",
            ".bin",
            ".skillpouch-file0001",
        ] {
            assert!(modules.join(kept).exists(), "{kept} was deleted");
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn leaves_what_a_link_points_to_alone() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let modules = root.path().join("node_modules");
        let bin = modules.join("skillpouch/bin");
        std::fs::create_dir_all(&bin)?;
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere)?;
        std::fs::write(elsewhere.join("keep"), b"")?;
        std::os::unix::fs::symlink(&elsewhere, modules.join(".skillpouch-Link0001"))?;

        tidy_next_to(&bin.join("skillpouch"));

        assert!(elsewhere.join("keep").exists());
        Ok(())
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
    fn finds_the_package_manager_windows_can_run() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let prefix = root.path().join("npm");
        let nodejs = root.path().join("nodejs");
        std::fs::create_dir_all(&prefix)?;
        std::fs::create_dir_all(&nodejs)?;
        // npm's global prefix holds only the shell script; Node's folder has both.
        std::fs::write(prefix.join("npm"), b"#!/bin/sh")?;
        std::fs::write(nodejs.join("npm"), b"#!/bin/sh")?;
        std::fs::write(nodejs.join("npm.cmd"), b"@echo off")?;
        let dirs = || [prefix.clone(), nodejs.clone()];

        assert_eq!(
            Manager::Npm.find(true, dirs()),
            Some(nodejs.join("npm.cmd"))
        );
        assert_eq!(Manager::Npm.find(false, dirs()), Some(prefix.join("npm")));
        assert_eq!(Manager::Pnpm.find(true, dirs()), None);
        // Not found anywhere: the name Windows would need.
        assert_eq!(Manager::Npm.files(true)[0], "npm.cmd");
        Ok(())
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
