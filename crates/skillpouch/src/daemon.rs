//! The background sync: syncs, then waits for a change on either side and
//! syncs again. Remote changes arrive through a long-poll on the pouch's
//! events; local ones are found by checking the pouch folder and the
//! assistants' folders every couple of seconds. A full sync also runs every
//! few minutes as a safety net.
//!
//! One runs per `~/.skillpouch` (a lock file sees to that), started as a
//! user service (systemd on Linux, launchd on macOS) or, where there is
//! none, as a detached process.

use crate::context::{Context, Session, VERSION};
use crate::sync::{self, Report};
use anyhow::{Context as _, Result};
use sp_core::home::user_home;
use sp_core::setup::SetupRecord;
use sp_harness::Places;
use sp_sync::status::{Phase, SyncStatus};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long the server holds an events request.
const LONG_POLL: u64 = 25;
/// How often local folders are checked for changes.
const LOCAL_CHECK: Duration = Duration::from_secs(2);
/// Changes settle this long before syncing (editors save in several steps).
const SETTLE: Duration = Duration::from_millis(500);
/// A full sync at least this often, whatever happens.
const FULL_SYNC: Duration = Duration::from_secs(5 * 60);
/// How often the background sync looks for a new SkillPouch release (the
/// registry itself is asked less often; see `update`).
const UPDATE_CHECK: Duration = Duration::from_secs(30 * 60);
/// The log is started over past this size.
const LOG_MAX: u64 = 1024 * 1024;

/// Which program runs the background sync, against which server: a
/// command that finds it older, moved or pointed elsewhere restarts it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub pid: u32,
    pub version: String,
    pub exe: PathBuf,
    pub api_url: String,
}

fn identity_path(ctx: &Context) -> PathBuf {
    ctx.home.run_dir().join("daemon.json")
}

impl Identity {
    fn ours(ctx: &Context) -> Option<Self> {
        Some(Self {
            pid: std::process::id(),
            version: VERSION.to_string(),
            exe: crate::update::exe()?,
            api_url: ctx.client.base().to_string(),
        })
    }

    pub fn load(ctx: &Context) -> Option<Self> {
        let bytes = fs::read(identity_path(ctx)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// This one should make way for one started by `ctx`'s program.
    pub fn outdated(&self, ctx: &Context) -> bool {
        self.api_url != ctx.client.base()
            || crate::update::newer(VERSION, &self.version)
            || !self.exe.exists()
    }
}

/// A running background sync that should be replaced: older than this
/// program, its program gone, or syncing with another server. One that
/// doesn't say (from before it was recorded) counts as outdated too.
pub fn outdated(ctx: &Context) -> bool {
    running(ctx) && Identity::load(ctx).is_none_or(|identity| identity.outdated(ctx))
}

fn lock_path(ctx: &Context) -> PathBuf {
    ctx.home.run_dir().join("daemon.lock")
}

/// Takes the daemon lock, or `None` when another daemon holds it.
fn try_lock(ctx: &Context) -> Result<Option<fs::File>> {
    let path = lock_path(ctx);
    ctx.home.ensure_private_dir(&ctx.home.run_dir())?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("couldn't open {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

/// A background sync is running for this `~/.skillpouch`.
pub fn running(ctx: &Context) -> bool {
    matches!(try_lock(ctx), Ok(None))
}

pub struct Log {
    path: PathBuf,
}

impl Log {
    pub fn new(ctx: &Context) -> Self {
        let _ = ctx.home.ensure_private_dir(&ctx.home.logs_dir());
        Self {
            path: ctx.home.logs_dir().join("sync.log"),
        }
    }

    pub fn line(&self, text: &str) {
        if fs::metadata(&self.path).is_ok_and(|meta| meta.len() > LOG_MAX) {
            let _ = fs::remove_file(&self.path);
        }
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{} {text}", sync::now());
        }
    }
}

pub fn describe_report(report: &Report) -> String {
    let mut parts = vec![format!(
        "synced: {} sent, {} received, {} conflicts, {} linked",
        report.sent, report.received, report.conflicts, report.linked
    )];
    if !report.removed.is_empty() {
        parts.push(format!(
            "removed (deleted in an assistant): {}",
            report.removed.join(", ")
        ));
    }
    if !report.backed_up.is_empty() {
        parts.push(format!("{} moved to backups", report.backed_up.len()));
    }
    for problem in &report.problems {
        parts.push(format!("note: {problem}"));
    }
    parts.join("; ")
}

/// `skillpouch daemon run`: syncs until stopped.
pub async fn run(ctx: &Context) -> Result<()> {
    let Some(_lock) = try_lock(ctx)? else {
        anyhow::bail!(
            "a background sync is already running for {}",
            ctx.home.root().display()
        );
    };
    let log = Log::new(ctx);
    log.line(&format!(
        "started (v{VERSION}, pid {}, {})",
        std::process::id(),
        ctx.client.base()
    ));
    set_pid(ctx, Some(std::process::id()));
    if let Some(identity) = Identity::ours(ctx)
        && let Ok(bytes) = serde_json::to_vec_pretty(&identity)
    {
        let _ = fs::write(identity_path(ctx), bytes);
    }
    let stopped = wait_for_stop();
    tokio::pin!(stopped);

    let mut failures: u32 = 0;
    let mut update_due = std::time::Instant::now();
    loop {
        if std::time::Instant::now() >= update_due {
            update_due = std::time::Instant::now() + UPDATE_CHECK;
            if crate::update::in_background(ctx, &log).await {
                set_pid(ctx, None);
                let _ = fs::remove_file(identity_path(ctx));
                drop(_lock);
                return restart_updated(&log);
            }
        }
        let session = ctx.session();
        let mut waiting_on: Option<(Session, Report)> = None;
        let pause = match session {
            Err(error) => {
                sync::mark(ctx, Phase::Stopped, None, Some(format!("{error:#}")));
                Duration::from_secs(30)
            }
            Ok(mut session) => match sync::run(ctx, &mut session).await {
                Ok(report) => {
                    failures = 0;
                    if report.sent + report.received + report.conflicts > 0 || report.first {
                        log.line(&describe_report(&report));
                    }
                    waiting_on = Some((session, report));
                    Duration::ZERO
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let (phase, message) = sync::describe(&error);
                    log.line(&format!("{phase:?}: {message}"));
                    match phase {
                        Phase::Offline => backoff(failures, 5, 60),
                        Phase::Stopped => Duration::from_secs(30),
                        _ => backoff(failures, 10, 300),
                    }
                }
            },
        };

        tell_news(ctx);
        let baseline = local_signature(ctx);
        let woke = async {
            match waiting_on {
                Some((mut session, report)) => {
                    tokio::select! {
                        () = remote_change(ctx, &mut session, &report) => "server",
                        () = local_change(ctx, baseline) => "this computer",
                        () = tokio::time::sleep(FULL_SYNC) => "timer",
                    }
                }
                None => {
                    tokio::select! {
                        () = local_change(ctx, baseline) => "this computer",
                        () = tokio::time::sleep(pause) => "retry",
                    }
                }
            }
        };
        tokio::select! {
            () = &mut stopped => break,
            _ = woke => {}
        }
        tokio::time::sleep(SETTLE).await;
    }
    set_pid(ctx, None);
    let _ = fs::remove_file(identity_path(ctx));
    log.line("stopped");
    Ok(())
}

/// Carries on as the version just installed: same process (the service
/// manager keeps track of it), new program.
fn restart_updated(log: &Log) -> Result<()> {
    let exe = crate::update::exe().context("couldn't find the skillpouch program")?;
    let mut command = std::process::Command::new(&exe);
    command.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        log.line(&format!("couldn't start the new version: {error}"));
        Err(error.into())
    }
    #[cfg(not(unix))]
    {
        let _ = log;
        command.spawn()?;
        Ok(())
    }
}

fn backoff(failures: u32, base: u64, max: u64) -> Duration {
    Duration::from_secs(base.saturating_mul(1 << failures.min(6)).min(max))
}

/// Desktop notification for what needs the person, once per thing.
fn tell_news(ctx: &Context) {
    let Some(mut status) = SyncStatus::load(&ctx.home) else {
        return;
    };
    if let Some((title, body)) = crate::notify::news(&mut status, ctx.client.base()) {
        let _ = status.save(&ctx.home);
        crate::notify::show(&title, &body);
    } else {
        let _ = status.save(&ctx.home);
    }
}

fn set_pid(ctx: &Context, pid: Option<u32>) {
    let mut status = SyncStatus::load(&ctx.home).unwrap_or(SyncStatus {
        phase: Phase::Idle,
        pouch_id: None,
        last_synced_at: None,
        message: None,
        sent: 0,
        received: 0,
        conflicts: 0,
        conflicts_notified: 0,
        problem_notified: None,
        daemon_pid: None,
        updated_at: sync::now(),
    });
    status.daemon_pid = pid;
    status.updated_at = sync::now();
    let _ = status.save(&ctx.home);
}

async fn wait_for_stop() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Returns when the server has something after the cursor (or can't say).
async fn remote_change(ctx: &Context, session: &mut Session, report: &Report) {
    loop {
        let Ok(token) = session.access_token(ctx, false).await else {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return;
        };
        let page = ctx
            .client
            .events(
                &session.key,
                &token,
                &report.pouch_id,
                &report.epoch,
                report.cursor,
                LONG_POLL,
            )
            .await;
        match page {
            Ok(page) if page.events.is_empty() && page.high_water <= report.cursor => {}
            // Something new, or the cursor/epoch moved under us: sync.
            Ok(_) => return,
            // Expired or replaced: wait on with a fresh token.
            Err(error) if error.code() == Some("token_invalid") => {
                if session.access_token(ctx, true).await.is_err() {
                    return;
                }
            }
            Err(error) if error.code().is_some() => return,
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(15)).await;
                return;
            }
        }
    }
}

/// Returns when anything the sync reads changes on this computer.
async fn local_change(ctx: &Context, baseline: u64) {
    loop {
        tokio::time::sleep(LOCAL_CHECK).await;
        if local_signature(ctx) != baseline {
            return;
        }
    }
}

/// A hash of names, sizes and times of everything the sync reads: the
/// pouch folders, the assistants' folders and config files, and this
/// computer's setup and login.
pub fn local_signature(ctx: &Context) -> u64 {
    let mut hasher = DefaultHasher::new();
    let mut add = |path: &Path, deep: bool| walk(path, deep, &mut hasher, 0);
    add(&ctx.home.root().join("pouches"), true);
    add(&ctx.home.state_dir().join("setup.json"), false);
    add(&ctx.home.credentials_file(), false);
    // `skillpouch select` and the TUI's Sync tab.
    add(&ctx.home.config_file(), false);
    if let Some(home) = user_home() {
        let places = Places::of(&home, ctx.home.root());
        for target in sp_harness::targets(&places) {
            if let Some(path) = &target.instructions {
                add(path, false);
            }
            for path in [
                target.mcp.as_ref().map(|(path, _)| path),
                target.hooks.as_ref().map(|(path, _)| path),
                target.plugins.as_ref().map(|(path, _)| path),
            ]
            .into_iter()
            .flatten()
            {
                add(path, false);
            }
            let commands = target.commands.as_ref().map(|(dir, _)| dir);
            let agents = target.agents.as_ref().map(|(dir, _)| dir);
            for dir in [target.skills.as_ref(), agents, commands]
                .into_iter()
                .flatten()
            {
                add(dir, true);
            }
        }
        // Skill folders that aren't linked into (another assistant's folder
        // is read instead) still have new skills brought into the pouch.
        for dir in [
            places.opencode.join("skills"),
            places.agents.join("skills"),
            places.codex.join("skills"),
        ] {
            add(&dir, true);
        }
    }
    hasher.finish()
}

fn walk(path: &Path, deep: bool, hasher: &mut DefaultHasher, depth: usize) {
    path.hash(hasher);
    let Ok(meta) = fs::symlink_metadata(path) else {
        0u8.hash(hasher);
        return;
    };
    meta.len().hash(hasher);
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .hash(hasher);
    // Links into the pouch are followed through the pouch folder itself.
    if !deep || !meta.is_dir() || depth > 8 {
        return;
    }
    let Ok(listing) = fs::read_dir(path) else {
        return;
    };
    let mut entries: Vec<PathBuf> = listing
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name().is_none_or(|name| {
                !sp_harness::IGNORED_DIRS.contains(&name.to_string_lossy().as_ref())
            })
        })
        .collect();
    entries.sort();
    for entry in entries {
        walk(&entry, deep, hasher, depth + 1);
    }
}

/// How the background sync was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    AlreadyRunning,
    /// Run from a temporary download (`npx`): nothing may point at it.
    Temporary,
    /// Installed as a user service: it also starts when you log in.
    Service,
    /// Started as a process of its own, until this computer restarts.
    Process,
}

/// Set when someone stops the background sync on purpose: it isn't started
/// again behind their back until they start it (or set the computer up).
fn stopped_marker(ctx: &Context) -> PathBuf {
    ctx.home.run_dir().join("stopped")
}

/// Starts the background sync if this computer is set up and it isn't
/// running (and wasn't stopped on purpose). Quiet: never fails a command.
pub fn ensure_running(ctx: &Context) {
    let set_up = ctx
        .credentials()
        .ok()
        .flatten()
        .is_some_and(|credentials| !SetupRecord::needed(&ctx.home, &credentials.device_id));
    if !set_up || stopped_marker(ctx).exists() {
        return;
    }
    if outdated(ctx) {
        let _ = stop(ctx);
    }
    if !running(ctx) {
        let _ = start(ctx);
    }
}

/// Makes sure a background sync runs, installing it as a service where
/// the system has one.
pub fn start(ctx: &Context) -> Result<Started> {
    let _ = fs::remove_file(stopped_marker(ctx));
    if running(ctx) {
        return Ok(Started::AlreadyRunning);
    }
    let exe = crate::update::exe().context("couldn't find the skillpouch program")?;
    if crate::update::temporary(&exe) {
        return Ok(Started::Temporary);
    }
    let installed = service::install(ctx, &exe).unwrap_or(false);
    if installed && service::STARTS_NOW {
        // Give the service a moment to take the lock.
        for _ in 0..20 {
            if running(ctx) {
                return Ok(Started::Service);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    spawn(ctx, &exe)?;
    Ok(if installed {
        Started::Service
    } else {
        Started::Process
    })
}

/// Starts `daemon run` as a process of its own, without a window.
pub fn spawn_detached(ctx: &Context) -> Result<()> {
    let exe = crate::update::exe().context("couldn't find the skillpouch program")?;
    spawn(ctx, &exe)
}

fn spawn(ctx: &Context, exe: &Path) -> Result<()> {
    let log = Log::new(ctx);
    let out = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log.path)?;
    let mut command = std::process::Command::new(exe);
    command
        .args(["daemon", "run", "--home"])
        .arg(ctx.home.root())
        .args(["--api-url", ctx.client.base()])
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: closing the terminal doesn't stop it.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // No console window, and closing the terminal doesn't stop it.
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    command
        .spawn()
        .context("couldn't start the background sync")?;
    Ok(())
}

/// Stops the background sync and keeps it from starting at login, until
/// it's started again (`skillpouch daemon start`, or setting up).
pub fn stop_on_purpose(ctx: &Context) -> Result<bool> {
    let stopped = stop(ctx)?;
    fs::create_dir_all(ctx.home.run_dir())?;
    fs::write(stopped_marker(ctx), b"")?;
    Ok(stopped)
}

/// Stops the background sync and keeps it from starting at login.
pub fn stop(ctx: &Context) -> Result<bool> {
    let had_service = service::uninstall().unwrap_or(false);
    if running(ctx)
        && let Some(pid) = SyncStatus::load(&ctx.home).and_then(|status| status.daemon_pid)
    {
        kill(pid);
        for _ in 0..30 {
            if !running(ctx) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(had_service || !running(ctx))
}

fn kill(pid: u32) {
    #[cfg(unix)]
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status();
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status();
}

mod service {
    //! Starting at login: a systemd user service (Linux), a launchd agent
    //! (macOS), or the user's `Run` registry key (Windows, no admin needed).

    use anyhow::Result;
    use std::path::Path;
    use std::process::{Command, Stdio};

    /// Settings passed on so the service uses the same folder. The server
    /// isn't: the service syncs with whichever one this computer logged in to.
    const PASSED_ON: [&str; 2] = ["HOME", "SKILLPOUCH_HOME"];

    fn quiet(command: &mut Command) -> bool {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[cfg(target_os = "linux")]
    fn unit_path() -> Option<std::path::PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| sp_core::home::user_home().map(|home| home.join(".config")))?;
        Some(config.join("systemd/user/skillpouch.service"))
    }

    #[cfg(target_os = "linux")]
    pub fn install(_ctx: &crate::context::Context, exe: &Path) -> Result<bool> {
        if !quiet(Command::new("systemctl").args(["--user", "show-environment"])) {
            return Ok(false);
        }
        let Some(path) = unit_path() else {
            return Ok(false);
        };
        let mut environment = String::new();
        for name in PASSED_ON {
            if let Ok(value) = std::env::var(name)
                && !value.is_empty()
            {
                environment.push_str(&format!("Environment=\"{name}={value}\"\n"));
            }
        }
        let unit = format!(
            "[Unit]\nDescription=SkillPouch sync\nAfter=network-online.target\n\n\
             [Service]\nExecStart=\"{}\" daemon run\nRestart=on-failure\nRestartSec=10\n{environment}\n\
             [Install]\nWantedBy=default.target\n",
            exe.display()
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, unit)?;
        Ok(
            quiet(Command::new("systemctl").args(["--user", "daemon-reload"]))
                && quiet(Command::new("systemctl").args([
                    "--user",
                    "enable",
                    "--now",
                    "skillpouch.service",
                ])),
        )
    }

    #[cfg(target_os = "linux")]
    pub fn uninstall() -> Result<bool> {
        let Some(path) = unit_path().filter(|path| path.exists()) else {
            return Ok(false);
        };
        quiet(Command::new("systemctl").args(["--user", "disable", "--now", "skillpouch.service"]));
        std::fs::remove_file(path)?;
        quiet(Command::new("systemctl").args(["--user", "daemon-reload"]));
        Ok(true)
    }

    #[cfg(target_os = "macos")]
    fn plist_path() -> Option<std::path::PathBuf> {
        sp_core::home::user_home()
            .map(|home| home.join("Library/LaunchAgents/net.skillpouch.sync.plist"))
    }

    #[cfg(target_os = "macos")]
    pub fn install(ctx: &crate::context::Context, exe: &Path) -> Result<bool> {
        let Some(path) = plist_path() else {
            return Ok(false);
        };
        let mut environment = String::new();
        for name in PASSED_ON {
            if let Ok(value) = std::env::var(name)
                && !value.is_empty()
            {
                environment.push_str(&format!("<key>{name}</key><string>{value}</string>"));
            }
        }
        let log = ctx.home.logs_dir().join("sync.log");
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\
             <key>Label</key><string>net.skillpouch.sync</string>\
             <key>ProgramArguments</key><array><string>{}</string><string>daemon</string><string>run</string></array>\
             <key>EnvironmentVariables</key><dict>{environment}</dict>\
             <key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\
             <key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string>\
             </dict></plist>\n",
            exe.display(),
            log.display(),
            log.display()
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, plist)?;
        let target = path.to_string_lossy().into_owned();
        quiet(Command::new("launchctl").args(["unload", &target]));
        Ok(quiet(
            Command::new("launchctl").args(["load", "-w", &target]),
        ))
    }

    #[cfg(target_os = "macos")]
    pub fn uninstall() -> Result<bool> {
        let Some(path) = plist_path().filter(|path| path.exists()) else {
            return Ok(false);
        };
        let target = path.to_string_lossy().into_owned();
        quiet(Command::new("launchctl").args(["unload", "-w", &target]));
        std::fs::remove_file(path)?;
        Ok(true)
    }

    /// Installing also starts it (else the caller starts it).
    pub const STARTS_NOW: bool = cfg!(any(target_os = "linux", target_os = "macos"));

    #[cfg(windows)]
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    #[cfg(windows)]
    const RUN_VALUE: &str = "SkillPouch";

    /// The command Windows runs at login.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn login_command(exe: &Path, home: &Path) -> String {
        format!(
            "\"{}\" daemon run --background --home \"{}\"",
            exe.display(),
            home.display()
        )
    }

    #[cfg(windows)]
    pub fn install(ctx: &crate::context::Context, exe: &Path) -> Result<bool> {
        let _ = PASSED_ON;
        let command = login_command(exe, ctx.home.root());
        Ok(quiet(Command::new("reg").args([
            "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f",
        ])))
    }

    #[cfg(windows)]
    pub fn uninstall() -> Result<bool> {
        let installed = quiet(Command::new("reg").args(["query", RUN_KEY, "/v", RUN_VALUE]));
        if installed {
            quiet(Command::new("reg").args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]));
        }
        Ok(installed)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    pub fn install(_ctx: &crate::context::Context, _exe: &Path) -> Result<bool> {
        let _ = (PASSED_ON, quiet as fn(&mut Command) -> bool);
        Ok(false)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    pub fn uninstall() -> Result<bool> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_login_command_quotes_its_paths() {
        let command = service::login_command(
            Path::new(r"C:\Users\Ada Lovelace\bin\skillpouch.exe"),
            Path::new(r"C:\Users\Ada Lovelace\.skillpouch"),
        );
        assert_eq!(
            command,
            r#""C:\Users\Ada Lovelace\bin\skillpouch.exe" daemon run --background --home "C:\Users\Ada Lovelace\.skillpouch""#
        );
    }

    #[test]
    fn an_older_moved_or_elsewhere_pointed_sync_makes_way() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ctx = Context::load_with(
            Some(dir.path().join(".skillpouch")),
            Some("https://skillpouch.net".into()),
        )?;
        let exe = crate::update::exe().context("no exe")?;
        let same = Identity {
            pid: 1,
            version: VERSION.into(),
            exe: exe.clone(),
            api_url: "https://skillpouch.net".into(),
        };
        assert!(!same.outdated(&ctx));
        let newer = Identity {
            version: "999.0.0".into(),
            ..same.clone()
        };
        assert!(!newer.outdated(&ctx));
        for other in [
            Identity {
                version: "0.0.1".into(),
                ..same.clone()
            },
            Identity {
                exe: dir.path().join("gone/skillpouch"),
                ..same.clone()
            },
            Identity {
                api_url: "http://localhost:5173".into(),
                ..same.clone()
            },
        ] {
            assert!(other.outdated(&ctx), "{other:?}");
        }
        Ok(())
    }

    #[test]
    fn backoff_grows_to_a_limit() {
        assert_eq!(backoff(1, 5, 60), Duration::from_secs(10));
        assert_eq!(backoff(3, 5, 60), Duration::from_secs(40));
        assert_eq!(backoff(30, 5, 60), Duration::from_secs(60));
    }

    #[test]
    fn the_signature_sees_new_and_changed_files() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut first = DefaultHasher::new();
        walk(dir.path(), true, &mut first, 0);
        fs::create_dir_all(dir.path().join("skills/pdf"))?;
        fs::write(dir.path().join("skills/pdf/SKILL.md"), "a")?;
        let mut second = DefaultHasher::new();
        walk(dir.path(), true, &mut second, 0);
        assert_ne!(first.finish(), second.finish());
        let mut again = DefaultHasher::new();
        walk(dir.path(), true, &mut again, 0);
        assert_eq!(second.finish(), again.finish());
        Ok(())
    }
}
