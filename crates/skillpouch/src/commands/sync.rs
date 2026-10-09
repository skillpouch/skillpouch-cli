//! `skillpouch sync` (once, in the foreground) and `skillpouch daemon …`
//! (the background sync).

use crate::context::Context;
use crate::daemon::{self, Started};
use crate::sync;
use crate::ui;
use anyhow::Result;
use sp_sync::status::{Phase, SyncStatus};
use std::process::ExitCode;

pub async fn once(ctx: &Context) -> Result<ExitCode> {
    let mut session = ctx.session()?;
    let spinner = ui::Spinner::start("Syncing…");
    let result = sync::run(ctx, &mut session).await;
    spinner.clear();
    let report = result?;
    let changed = report.sent + report.received + report.conflicts;
    if changed == 0 && !report.first {
        ui::success("Up to date.");
    } else {
        ui::success(format!(
            "Synced: {} sent, {} received{}.",
            report.sent,
            report.received,
            if report.conflicts > 0 {
                format!(", {} changed in two places", report.conflicts)
            } else {
                String::new()
            }
        ));
    }
    if let Some(open) = report.open_conflicts.filter(|open| *open > 0) {
        let url = format!(
            "{}{}",
            ctx.client.base(),
            crate::sync::conflicts_path(&report.pouch_id)
        );
        ui::warn(format!(
            "{} changed in two places. Pick the version to keep: {}",
            crate::sync::conflict_words(open),
            ui::accent(&url)
        ));
        ui::hint("Or run `skillpouch conflicts` to open it.");
    }
    if !report.removed.is_empty() {
        ui::hint(format!(
            "Deleted in an assistant, so removed from the pouch: {}",
            report.removed.join(", ")
        ));
    }
    if !report.backed_up.is_empty() {
        ui::hint(format!(
            "{} file(s) the pouch replaced were moved to {}",
            report.backed_up.len(),
            ctx.home.root().join("backups").display()
        ));
    }
    for problem in &report.problems {
        ui::warn(problem);
    }
    if !daemon::running(ctx) {
        ui::hint("Background sync is off because it was stopped: `skillpouch daemon start`.");
    }
    Ok(ExitCode::SUCCESS)
}

pub async fn run_daemon(ctx: &Context, background: bool) -> Result<ExitCode> {
    if background && cfg!(windows) {
        // Started at login with a console window: start again without one.
        daemon::spawn_detached(ctx)?;
        return Ok(ExitCode::SUCCESS);
    }
    daemon::run(ctx).await?;
    Ok(ExitCode::SUCCESS)
}

pub fn start(ctx: &Context) -> Result<ExitCode> {
    match daemon::start(ctx)? {
        Started::AlreadyRunning => ui::success("Background sync is already running."),
        Started::Service => {
            ui::success("Background sync started. It starts again when you log in.");
        }
        Started::Process => {
            ui::success("Background sync started.");
            ui::hint("It stops when this computer restarts; `skillpouch` starts it again.");
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn stop(ctx: &Context) -> Result<ExitCode> {
    if daemon::stop_on_purpose(ctx)? {
        ui::success("Background sync stopped.");
    } else {
        ui::warn("Background sync wasn't running.");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn status(ctx: &Context) -> Result<ExitCode> {
    let running = daemon::running(ctx);
    let status = SyncStatus::load(&ctx.home);
    println!(
        "{}  {}",
        ui::dim("Background"),
        if running { "running" } else { "not running" }
    );
    if let Some(status) = status {
        let phase = match status.phase {
            Phase::Idle => "up to date",
            Phase::Syncing => "syncing",
            Phase::Offline => "offline",
            Phase::Failed => "failed",
            Phase::Stopped => "stopped",
        };
        println!("{}       {phase}", ui::dim("State"));
        if let Some(at) = &status.last_synced_at {
            println!("{}   {at}", ui::dim("Last sync"));
        }
        if let Some(message) = &status.message {
            println!("{}     {message}", ui::dim("Note"));
        }
    }
    println!(
        "{}         {}",
        ui::dim("Log"),
        ctx.home.logs_dir().join("sync.log").display()
    );
    Ok(ExitCode::SUCCESS)
}

/// `skillpouch conflicts`: opens the web app where conflicts are resolved.
pub fn conflicts(ctx: &Context) -> Result<ExitCode> {
    let record = sp_core::setup::SetupRecord::load(&ctx.home)?.ok_or(sync::Stopped::NotSetUp)?;
    let url = format!(
        "{}{}",
        ctx.client.base(),
        sync::conflicts_path(&record.pouch_id)
    );
    let open = SyncStatus::load(&ctx.home).map_or(0, |status| status.conflicts);
    if open == 0 {
        ui::success("No conflicts in this computer's pouch.");
    }
    if crate::browser::open(&url) {
        ui::hint(format!("Opened {}", ui::accent(&url)));
    } else {
        ui::hint(format!("Open {}", ui::accent(&url)));
    }
    Ok(ExitCode::SUCCESS)
}
