//! One sync of this computer's pouch, start to finish: bring in what the
//! assistants have (all of it on the first sync, as chosen at setup; only
//! new things after that), sync the pouch folder with the server, then link
//! the pouch into every assistant.

use crate::context::{Context, Session};
use anyhow::{Context as _, Result};
use sp_core::home::user_home;
use sp_core::setup::{FirstSync, SetupRecord};
use sp_harness::{Harness, Places, Target};
use sp_sync::SyncError;
use sp_sync::adopt::{self, Found};
use sp_sync::engine::{self, Options, Prefer};
use sp_sync::hooks::{self, HookFile};
use sp_sync::item::PouchDir;
use sp_sync::item::{File, Files};
use sp_sync::plugins::{self, PluginFile};
use sp_sync::project::{self, Projection};
use sp_sync::remote::Remote;
use sp_sync::state::PouchState;
use sp_sync::status::{Phase, SyncStatus};
use std::path::PathBuf;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Rounds of the engine per sync when each round turns up more to do.
const ROUNDS: usize = 3;

/// What one sync did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub pouch_id: String,
    pub sent: usize,
    pub received: usize,
    pub conflicts: usize,
    /// Conflicts in the pouch waiting for someone to pick a version on the
    /// web (`None`: couldn't ask).
    pub open_conflicts: Option<usize>,
    pub linked: usize,
    pub backed_up: Vec<PathBuf>,
    /// Things left alone, each with why.
    pub problems: Vec<String>,
    /// The cursor reached: events after it are new.
    pub cursor: u64,
    pub epoch: String,
    pub first: bool,
    /// Items deleted in an assistant, so taken out of the pouch (`skill pdf`).
    pub removed: Vec<String>,
    /// Items the server no longer had though nobody deleted them, sent
    /// again from here (`skill/pdf`).
    pub reuploaded: Vec<String>,
}

/// Why nothing was synced.
#[derive(Debug, thiserror::Error)]
pub enum Stopped {
    #[error("This computer isn't set up yet: run `skillpouch` and choose a pouch.")]
    NotSetUp,
    #[error(
        "The pouch is locked: your plan has fewer pouches than you have. Unlock it on the web."
    )]
    Locked,
    #[error(
        "The pouch this computer synced was deleted. Its items stay on this computer; run `skillpouch` to choose another pouch."
    )]
    PouchDeleted,
}

/// The dashboard's sync line: from the status file, whether the
/// background sync runs, and whether this computer is set up at all.
pub fn view(
    status: Option<&SyncStatus>,
    running: bool,
    set_up: bool,
    now: OffsetDateTime,
) -> Option<sp_tui::SyncView> {
    use sp_tui::{SyncTone, SyncView};
    if !set_up {
        return None;
    }
    let last = status
        .and_then(|status| status.last_synced_at.as_deref())
        .and_then(|at| crate::when::ago(at, now));
    let line = |tone, text: String, detail: Option<String>| SyncView {
        tone,
        text,
        detail,
        conflicts: None,
    };
    let not_running =
        || Some("Background sync isn't running: `skillpouch daemon start`".to_string());
    let Some(status) = status else {
        return Some(if running {
            line(SyncTone::Busy, "Starting…".into(), None)
        } else {
            line(SyncTone::Warn, "Not synced yet".into(), not_running())
        });
    };
    let first_line = |text: &str| {
        status
            .message
            .as_deref()
            .map(|m| m.lines().next().unwrap_or(m).to_string())
            .or_else(|| (!text.is_empty()).then(|| text.to_string()))
    };
    Some(match status.phase {
        Phase::Syncing if running => line(SyncTone::Busy, "Syncing…".into(), None),
        Phase::Offline => line(
            SyncTone::Warn,
            "Offline".into(),
            Some(match &last {
                Some(last) => format!("Can't reach the server · last synced {last}"),
                None => "Can't reach the server; changes wait on this computer".into(),
            }),
        ),
        Phase::Failed => line(SyncTone::Bad, "Sync failed".into(), first_line("")),
        Phase::Stopped => line(SyncTone::Warn, "Not syncing".into(), first_line("")),
        _ if !running => line(
            SyncTone::Warn,
            last.map_or("Not synced yet".into(), |last| format!("Synced {last}")),
            not_running(),
        ),
        _ if status.conflicts > 0 => SyncView {
            conflicts: status.pouch_id.as_deref().map(conflicts_path),
            ..line(
                SyncTone::Warn,
                last.map_or("Up to date".into(), |last| format!("Synced {last}")),
                Some(format!(
                    "{} changed in two places: press c to pick versions in the web app",
                    conflict_words(status.conflicts)
                )),
            )
        },
        _ => line(
            SyncTone::Good,
            last.map_or("Up to date".into(), |last| format!("Synced {last}")),
            status
                .message
                .as_deref()
                .map(|m| m.lines().next().unwrap_or(m).to_string()),
        ),
    })
}

/// Makes `pouch_id` the pouch this computer syncs from now on.
pub fn switch_pouch(ctx: &Context, device_id: &str, pouch_id: &str) -> Result<()> {
    if let Some(mut record) = SetupRecord::load(&ctx.home)?
        && record.device_id == device_id
        && record.pouch_id != pouch_id
    {
        record.pouch_id = pouch_id.to_string();
        record.save(&ctx.home)?;
    }
    Ok(())
}

pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

pub fn pouch_dir(ctx: &Context, pouch_id: &str) -> PathBuf {
    ctx.home.root().join("pouches").join(pouch_id)
}

pub fn places(ctx: &Context) -> Result<Places> {
    let home = user_home().context("couldn't find your home folder")?;
    Ok(Places::of(&home, ctx.home.root()))
}

fn backups_dir(ctx: &Context) -> PathBuf {
    let stamp = OffsetDateTime::now_utc()
        .format(time::macros::format_description!(
            "[year][month][day]-[hour][minute][second]"
        ))
        .unwrap_or_default();
    ctx.home.root().join("backups").join(stamp)
}

/// Where the web app resolves a pouch's conflicts.
pub fn conflicts_path(pouch_id: &str) -> String {
    format!("/app/p/{pouch_id}/conflicts")
}

/// `1 item`, `3 items`.
pub fn conflict_words(count: usize) -> String {
    if count == 1 {
        "1 item".into()
    } else {
        format!("{count} items")
    }
}

/// Syncs once. The status file says what happened either way.
pub async fn run(ctx: &Context, session: &mut Session) -> Result<Report> {
    let record = SetupRecord::load(&ctx.home)?
        .filter(|record| record.device_id == session.credentials.device_id)
        .ok_or(Stopped::NotSetUp)?;
    let lock = crate::choose::sync_lock(ctx);
    let _lock = tokio::task::spawn_blocking(move || sp_core::home::lock_exclusive(&lock))
        .await
        .context("couldn't wait for another sync")??;
    // Unlinked while waiting for the lock.
    let record = SetupRecord::load(&ctx.home)?
        .filter(|saved| saved.device_id == record.device_id)
        .ok_or(Stopped::NotSetUp)?;
    mark(ctx, Phase::Syncing, Some(&record.pouch_id), None);
    let mut result = run_with(ctx, session, record.clone()).await;
    // The token ran out or was replaced mid-sync: a fresh one, then once more.
    if result.as_ref().is_err_and(token_rejected) {
        session.access_token(ctx, true).await?;
        result = run_with(ctx, session, record).await;
    }
    match &result {
        Ok(report) => {
            let mut status = base_status(ctx, Phase::Idle, Some(&report.pouch_id));
            status.last_synced_at = Some(now());
            status.message = (!report.problems.is_empty()).then(|| report.problems.join("\n"));
            if report.sent + report.received + report.conflicts > 0 {
                status.sent = report.sent;
                status.received = report.received;
            }
            if let Some(open) = report.open_conflicts {
                status.conflicts = open;
            }
            let _ = status.save(&ctx.home);
        }
        Err(error) => {
            let (phase, message) = describe(error);
            mark(ctx, phase, None, Some(message));
        }
    }
    result
}

/// The server refused the access token (expired, or replaced by a refresh
/// in another process); a fresh one fixes it.
fn token_rejected(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<SyncError>()
            .and_then(SyncError::code)
            .or_else(|| {
                cause
                    .downcast_ref::<sp_api::ApiError>()
                    .and_then(sp_api::ApiError::code)
            })
            == Some("token_invalid")
    })
}

/// The phase and words for a failed sync.
pub fn describe(error: &anyhow::Error) -> (Phase, String) {
    if let Some(stopped) = error.downcast_ref::<Stopped>() {
        return (Phase::Stopped, stopped.to_string());
    }
    let offline = error.chain().any(|cause| {
        cause
            .downcast_ref::<SyncError>()
            .is_some_and(SyncError::offline)
    }) || error.chain().any(|cause| {
        cause
            .downcast_ref::<sp_api::ApiError>()
            .is_some_and(|e| matches!(e, sp_api::ApiError::Network(_)))
    });
    if offline {
        return (
            Phase::Offline,
            "Can't reach the server. Changes wait here and sync when it's back.".into(),
        );
    }
    if let Some(exit) = error.downcast_ref::<crate::exit::Exit>() {
        return (Phase::Stopped, exit.message.clone());
    }
    (Phase::Failed, format!("{error:#}"))
}

fn base_status(ctx: &Context, phase: Phase, pouch_id: Option<&str>) -> SyncStatus {
    let old = SyncStatus::load(&ctx.home);
    SyncStatus {
        phase,
        pouch_id: pouch_id
            .map(str::to_string)
            .or_else(|| old.as_ref().and_then(|s| s.pouch_id.clone())),
        last_synced_at: old.as_ref().and_then(|s| s.last_synced_at.clone()),
        message: None,
        sent: old.as_ref().map_or(0, |s| s.sent),
        received: old.as_ref().map_or(0, |s| s.received),
        conflicts: old.as_ref().map_or(0, |s| s.conflicts),
        conflicts_notified: old.as_ref().map_or(0, |s| s.conflicts_notified),
        problem_notified: old.as_ref().and_then(|s| s.problem_notified.clone()),
        daemon_pid: old.as_ref().and_then(|s| s.daemon_pid),
        updated_at: now(),
    }
}

/// Records the phase, keeping what the status file already says.
pub fn mark(ctx: &Context, phase: Phase, pouch_id: Option<&str>, message: Option<String>) {
    let mut status = base_status(ctx, phase, pouch_id);
    status.message = message;
    let _ = status.save(&ctx.home);
}

async fn run_with(ctx: &Context, session: &mut Session, mut record: SetupRecord) -> Result<Report> {
    let token = session.access_token(ctx, false).await?;
    let remote = Remote::open(
        &ctx.client,
        &session.key,
        &token,
        &session.credentials,
        &record.pouch_id,
        &ctx.home,
    )
    .await
    .map_err(api_signed_out)?;
    if remote.pouch.state == "locked" {
        return Err(Stopped::Locked.into());
    }

    let root = pouch_dir(ctx, &record.pouch_id);
    ctx.home.ensure_private_dir(&root)?;
    let dir = PouchDir::new(&root);
    let places = places(ctx)?;
    let backups = backups_dir(ctx);
    let mut projection = Projection::load(&ctx.home);
    if !record.done {
        // Set up afresh (a new login, another account or server): what the
        // assistants have from the pouch used so far is theirs again, and the
        // first sync treats it like anything else they have.
        let places = places.clone();
        let targets = tokio::task::spawn_blocking(move || sp_harness::targets(&places)).await?;
        sp_sync::release::leave_pouch(&ctx.home, &mut projection, &targets, &root)?;
        projection.save(&ctx.home)?;
    }
    project::set_current(&ctx.home, &root)?;
    // Edits made in copies (Gemini's TOML commands, Windows without links)
    // go into the pouch folder first, so this sync sends them.
    projection.pull_copies()?;
    let mut report = Report {
        pouch_id: record.pouch_id.clone(),
        first: !record.done,
        ..Report::default()
    };

    let (harnesses, all_targets) = {
        let places = places.clone();
        tokio::task::spawn_blocking(move || {
            (sp_harness::discover(&places), sp_harness::targets(&places))
        })
        .await?
    };
    // What this computer syncs (read each time: `skillpouch select` and the
    // TUI change it while the background sync runs).
    let choice = sp_core::Config::load(&ctx.home)
        .map(|config| config.sync)
        .unwrap_or_default();
    release_unchosen(&ctx.home, &mut projection, &all_targets, &choice)?;
    let targets: Vec<Target> = all_targets
        .into_iter()
        .filter(|target| choice.assistant_on(target.id))
        .collect();
    let harnesses: Vec<Harness> = harnesses
        .into_iter()
        .filter(|harness| choice.assistant_on(harness.id))
        .map(|harness| chosen_items(harness, &choice))
        .collect();
    if record.done {
        follow_assistants(
            &ctx.home,
            &dir,
            &targets,
            &mut projection,
            &backups,
            &mut report,
        )?;
    }
    let mut options = if record.done {
        bring_in_new(
            &dir,
            &harnesses,
            &targets,
            &mut projection,
            &backups,
            &mut report,
        )?;
        Options::default()
    } else {
        first_sync(
            &dir,
            &record.first_sync,
            &harnesses,
            &targets,
            &mut projection,
            &backups,
            &mut report,
        )?
    };
    if !choice.skip_kinds.is_empty() {
        // Kinds left out here aren't on this computer: not a reason to
        // delete them from the pouch.
        options.delete_unmatched = false;
    }
    let claude_plugins = plugins::known_marketplaces(&places.claude);
    let config_targets: Vec<Target> = targets
        .iter()
        .cloned()
        .map(|mut target| {
            if !choice.kind_on("hook") {
                target.hooks = None;
            }
            if !choice.kind_on("plugin") {
                target.plugins = None;
            }
            target
        })
        .collect();
    bring_in_hooks_and_plugins(&dir, &config_targets, &mut projection, &claude_plugins)?;

    let mut settings_state = sp_sync::settings::DeviceState::load(&ctx.home, &record.pouch_id)?;
    // Uninstalled assistants lose their per-device preferences history, but
    // an explicit `select --off` remains in the user's config.toml.
    let installed: std::collections::BTreeSet<_> = sp_harness::targets(&places)
        .into_iter()
        .map(|target| target.id.to_string())
        .collect();
    settings_state
        .harnesses
        .retain(|id, _| installed.contains(id));
    if choice.kind_on("setting") {
        sp_sync::settings::capture(&dir, &targets, &mut settings_state)?;
        settings_state.save(&ctx.home, &record.pouch_id)?;
    }

    let previously_present: std::collections::BTreeSet<_> = dir
        .read()?
        .0
        .keys()
        .filter(|key| key.starts_with("setting/"))
        .cloned()
        .collect();
    let mut state = PouchState::load(&ctx.home, &record.pouch_id)?;
    if state.epoch != remote.pouch.epoch && !state.epoch.is_empty() {
        // The pouch's history was rewritten: compare everything afresh.
        state.restart();
    }
    for round in 0..ROUNDS {
        let outcome = engine::sync_with_checkpoint(
            &remote,
            &dir,
            &mut state,
            if round == 0 {
                options
            } else {
                Options::default()
            },
            |state| state.save(&ctx.home, &record.pouch_id),
        )
        .await
        .map_err(api_signed_out)?;
        state.save(&ctx.home, &record.pouch_id)?;
        report.sent += outcome.sent;
        report.received += outcome.received;
        report.conflicts += outcome.conflicts;
        report.problems.extend(outcome.rejected);
        report.reuploaded.extend(outcome.reuploaded);
        if outcome.unreadable > 0 && round == 0 {
            report.problems.push(format!(
                "{} item(s) in the pouch couldn't be verified and were left out",
                outcome.unreadable
            ));
        }
        if round == 0 {
            for path in outcome.skipped.too_large {
                report
                    .problems
                    .push(format!("{path} is too large to sync (1 MB per file)"));
            }
            for path in outcome.skipped.bad_names {
                report.problems.push(format!(
                    "{path}: names may only use letters, digits, `.`, `_` and `-`"
                ));
            }
        }
        if !outcome.again {
            break;
        }
    }

    let (mut items, _) = dir.read()?;
    if choice.kind_on("setting") {
        for target in &targets {
            let key = format!("setting/{}", target.id);
            if !items.contains_key(&key) && previously_present.contains(&key) {
                settings_state.deleted.insert(target.id.into());
            }
        }
        report.backed_up.extend(sp_sync::settings::project(
            &dir,
            &items,
            &targets,
            &mut settings_state,
            &backups,
        )?);
        settings_state.save(&ctx.home, &record.pouch_id)?;
    }
    items.retain(|_, item| choice.kind_on(&item.kind));
    let (next, projected) = project::project(&ctx.home, &items, &targets, &backups, &projection)?;
    projection.links = next.links;
    projection.mcp = next.mcp;
    projection.copies = next.copies;
    projection.hooks = next.hooks;
    projection.plugins = next.plugins;
    projection.save(&ctx.home)?;
    report.linked = projected.linked;
    report.backed_up.extend(projected.backed_up);
    report.problems.extend(projected.problems);

    if !record.done {
        record.done = true;
        record.save(&ctx.home)?;
    }
    report.cursor = state.cursor;
    report.epoch = state.epoch.clone();
    report.open_conflicts = remote.open_conflicts().await.ok();
    // Only for the web's device list; a failure here doesn't matter.
    let _ = remote.acknowledge(state.cursor).await;
    Ok(report)
}

/// Hands back to the assistants what this computer no longer syncs: the
/// assistants turned off here, and the kinds turned off.
pub fn release_unchosen(
    home: &sp_core::Home,
    projection: &mut Projection,
    targets: &[Target],
    choice: &sp_core::SyncChoice,
) -> Result<sp_sync::release::Released> {
    Ok(sp_sync::release::release(
        home,
        projection,
        targets,
        &|owner, kind| owner.is_some_and(|id| !choice.assistant_on(id)) || !choice.kind_on(kind),
    )?)
}

/// Leaves out of `harness` the kinds this computer doesn't sync.
fn chosen_items(mut harness: Harness, choice: &sp_core::SyncChoice) -> Harness {
    harness
        .items
        .retain(|item| choice.kind_on(kind_name(item.kind)));
    harness
}

/// The pouch's name for a kind found in an assistant.
pub fn kind_name(kind: sp_harness::Kind) -> &'static str {
    match kind {
        sp_harness::Kind::Instructions => "instructions",
        sp_harness::Kind::Skill => "skill",
        sp_harness::Kind::Agent => "agent",
        sp_harness::Kind::Command => "command",
        sp_harness::Kind::McpServer => "mcp_server",
    }
}

/// The first sync, as chosen at setup.
fn first_sync(
    dir: &PouchDir,
    choice: &FirstSync,
    harnesses: &[Harness],
    targets: &[Target],
    projection: &mut Projection,
    backups: &std::path::Path,
    report: &mut Report,
) -> Result<Options> {
    let found = |id: &str| -> Vec<Found> {
        harnesses
            .iter()
            .filter(|harness| harness.id == id)
            .flat_map(adopt::found_in)
            .collect()
    };
    let others = |id: Option<&str>| -> Vec<Found> {
        harnesses
            .iter()
            .filter(|harness| Some(harness.id) != id)
            .flat_map(adopt::found_in)
            .collect()
    };
    let mut adopt_all = |chosen: &str, replace: bool| -> Result<()> {
        let adopted = adopt::adopt(dir, &found(chosen), replace, Some(backups))?;
        report.backed_up.extend(adopted.backed_up);
        Ok(())
    };
    let options = match choice {
        FirstSync::Fill {
            assistant: Some(id),
        }
        | FirstSync::Merge { assistant: id } => {
            adopt_all(id, false)?;
            // The other assistants join in; differing copies are kept side by side.
            let adopted = adopt::adopt(dir, &others(Some(id)), false, Some(backups))?;
            report.backed_up.extend(adopted.backed_up);
            Options::default()
        }
        FirstSync::UseComputer { assistant: id } => {
            adopt_all(id, true)?;
            report
                .backed_up
                .extend(adopt::set_aside(&others(Some(id)), backups)?);
            ignore_servers(projection, targets);
            leave_hooks_and_plugins(projection, targets, Some(id));
            Options {
                prefer: Prefer::Computer,
                delete_unmatched: true,
            }
        }
        FirstSync::UsePouch | FirstSync::Fill { assistant: None } => {
            report
                .backed_up
                .extend(adopt::set_aside(&others(None), backups)?);
            ignore_servers(projection, targets);
            leave_hooks_and_plugins(projection, targets, None);
            Options::default()
        }
    };
    Ok(options)
}

/// MCP servers already in the configs that weren't brought into the pouch:
/// left where they are and never adopted later.
fn ignore_servers(projection: &mut Projection, targets: &[Target]) {
    for (name, _) in projection.foreign_servers(targets) {
        for (path, _) in targets.iter().filter_map(|target| target.mcp.as_ref()) {
            projection
                .ignored
                .entry(path.clone())
                .or_default()
                .insert(name.clone());
        }
    }
}

/// Hooks and plugins of assistants that weren't brought into the pouch at
/// setup: left where they are, and not brought in later either.
fn leave_hooks_and_plugins(projection: &mut Projection, targets: &[Target], except: Option<&str>) {
    for target in targets.iter().filter(|target| Some(target.id) != except) {
        if let Some((path, format)) = &target.hooks {
            projection
                .hooks
                .entry(path.clone())
                .or_insert_with(|| HookFile {
                    written: Vec::new(),
                    seen: hooks::read(path, *format),
                });
        }
        if let Some((path, format)) = &target.plugins {
            projection
                .plugins
                .entry(path.clone())
                .or_insert_with(|| PluginFile {
                    seen: plugins::turned_on(path, *format),
                    ..PluginFile::default()
                });
        }
    }
}

/// A name not taken in the pouch: `stop-say`, else `stop-say-2`, …
fn free_name(dir: &PouchDir, kind: &str, wanted: &str) -> String {
    let taken = |name: &str| dir.item_dir(kind, name).is_some_and(|path| path.exists());
    if !taken(wanted) {
        return wanted.to_string();
    }
    (2..)
        .map(|n| format!("{wanted}-{n}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| wanted.to_string())
}

fn save_item(dir: &PouchDir, kind: &str, name: &str, files: Files) -> Result<()> {
    dir.write_item(kind, name, &files)?;
    Ok(())
}

/// Hooks and plugins set up in an assistant by hand (or edited there) go
/// into the pouch.
fn bring_in_hooks_and_plugins(
    dir: &PouchDir,
    targets: &[Target],
    projection: &mut Projection,
    known_marketplaces: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<()> {
    let (existing, _) = dir.read()?;
    let mut have: Vec<hooks::Hook> = existing
        .values()
        .filter(|item| item.kind == "hook")
        .filter_map(|item| hooks::parse_hook(&item.files.get("hook.json")?.bytes).ok())
        .collect();
    let mut have_plugins: Vec<plugins::Plugin> = existing
        .values()
        .filter_map(|item| match plugins::of(item)? {
            plugins::Kind::Reference(plugin) => Some(plugin),
            plugins::Kind::Folder { .. } => None,
        })
        .collect();
    for target in targets {
        if let Some((path, format)) = &target.hooks {
            let changes = hooks::changes(path, *format, projection.hooks.get(path));
            for (name, edited) in changes.edited {
                let Some(item) = existing.get(&sp_sync::item::key("hook", &name)) else {
                    continue;
                };
                let old = item
                    .files
                    .get("hook.json")
                    .and_then(|file| hooks::parse_hook(&file.bytes).ok());
                let hook = hooks::Hook {
                    name: name.clone(),
                    description: old.map(|old| old.description).unwrap_or_default(),
                    ..edited
                };
                let files: Files =
                    [("hook.json".to_string(), File::new(hooks::hook_file(&hook)))].into();
                save_item(dir, "hook", &item.name, files)?;
                have.push(hook);
            }
            for found in changes.added {
                if have.iter().any(|hook| hook.same_action(&found.hook)) {
                    continue;
                }
                let name = free_name(dir, "hook", &hooks::name_for(&found.hook));
                let hook = hooks::Hook {
                    name: name.clone(),
                    ..found.hook
                };
                let files: Files =
                    [("hook.json".to_string(), File::new(hooks::hook_file(&hook)))].into();
                save_item(dir, "hook", &name, files)?;
                have.push(hook);
                // The entry is the pouch's from now on: changing the hook
                // anywhere replaces it here too.
                if let (Some((event, entry)), None) = (found.entry, &target.read_only) {
                    let file = projection.hooks.entry(path.clone()).or_default();
                    file.written.push(hooks::WrittenHook { name, event, entry });
                }
            }
        }
        if let Some((path, format)) = &target.plugins {
            for found in plugins::added(
                path,
                *format,
                projection.plugins.get(path),
                known_marketplaces,
            ) {
                let same = |plugin: &plugins::Plugin| {
                    (found.source.is_some() && plugin.source == found.source)
                        || (found.npm.is_some() && plugin.npm == found.npm)
                };
                if have_plugins.iter().any(same) {
                    continue;
                }
                let name = free_name(dir, "plugin", &found.name);
                let plugin = plugins::Plugin {
                    name: name.clone(),
                    ..found
                };
                let files: Files =
                    [("plugin.json".to_string(), plugins::plugin_file(&plugin))].into();
                save_item(dir, "plugin", &name, files)?;
                have_plugins.push(plugin);
            }
        }
    }
    Ok(())
}

/// Items deleted in an assistant leave the pouch (moved to the backups
/// first); items replaced there take the new content.
fn follow_assistants(
    home: &sp_core::Home,
    dir: &PouchDir,
    targets: &[Target],
    projection: &mut Projection,
    backups: &std::path::Path,
    report: &mut Report,
) -> Result<()> {
    let noticed = sp_sync::noticed::noticed(home, dir, targets, projection)?;
    for replaced in noticed.replaced {
        dir.write_item(&replaced.kind, &replaced.name, &replaced.files)?;
    }
    for removed in noticed.removed {
        if let Some(item) = dir.item_dir(&removed.kind, &removed.name)
            && item.exists()
        {
            report.backed_up.push(project::back_up(&item, backups)?);
        }
        projection
            .dismissed
            .insert(sp_sync::item::key(&removed.kind, &removed.name));
        report
            .removed
            .push(format!("{} {}", noun(&removed.kind), removed.name));
    }
    report.problems.extend(noticed.held_back);
    Ok(())
}

fn noun(kind: &str) -> &str {
    match kind {
        "mcp_server" => "MCP server",
        other => other,
    }
}

/// After the first sync: things the user added to an assistant directly,
/// and MCP servers edited in an assistant's config, go into the pouch.
fn bring_in_new(
    dir: &PouchDir,
    harnesses: &[Harness],
    targets: &[Target],
    projection: &mut Projection,
    backups: &std::path::Path,
    report: &mut Report,
) -> Result<()> {
    let (existing, _) = dir.read()?;
    // Back in the pouch (from the web, say): no longer dismissed.
    projection
        .dismissed
        .retain(|key| !existing.contains_key(key));
    let projection = &*projection;
    let item_key = |item: &sp_sync::item::LocalItem| sp_sync::item::key(&item.kind, &item.name);
    let mut found: Vec<Found> = Vec::new();
    for harness in harnesses {
        let config = targets
            .iter()
            .find(|target| target.id == harness.id)
            .and_then(|target| target.mcp.as_ref())
            .map(|(path, _)| path.clone());
        let skip = |name: &str| {
            config.as_ref().is_some_and(|path| {
                projection
                    .mcp
                    .get(path)
                    .is_some_and(|ours| ours.contains_key(name))
                    || projection
                        .ignored
                        .get(path)
                        .is_some_and(|ignored| ignored.contains(name))
            })
        };
        // A folder in a git checkout keeps its files, so they'd be found
        // again on every sync: only names the pouch doesn't have come in.
        let read_only = sp_harness::git_checkout(&harness.root).is_some();
        found.extend(adopt::found_in(harness).into_iter().filter(|f| {
            let ours = f
                .origin
                .as_ref()
                .is_some_and(|origin| projection.copies.contains_key(origin));
            let key = item_key(&f.item);
            let known =
                read_only && (existing.contains_key(&key) || projection.dismissed.contains(&key));
            !ours && !known && (f.item.kind != "mcp_server" || !skip(&f.item.name))
        }));
    }
    let adopted = adopt::adopt(dir, &found, false, Some(backups))?;
    report.backed_up.extend(adopted.backed_up);

    let edited: Vec<Found> = projection
        .edited_servers(targets)
        .into_iter()
        .filter_map(|(name, server)| {
            let mut text = serde_json::to_string_pretty(&server).ok()?;
            text.push('\n');
            let mut files = sp_sync::item::Files::new();
            files.insert("server.json".into(), sp_sync::item::File::new(text));
            Some(Found {
                item: sp_sync::item::LocalItem {
                    kind: "mcp_server".into(),
                    name,
                    files,
                },
                origin: None,
            })
        })
        .collect();
    adopt::adopt(dir, &edited, true, None)?;
    Ok(())
}

/// Turns "this device can't sign in any more" into the not-logged-in exit.
fn api_signed_out(error: SyncError) -> anyhow::Error {
    match error {
        SyncError::Api(api) if api.code().is_some() => crate::context::signed_out(*api),
        SyncError::PouchDeleted => Stopped::PouchDeleted.into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_tui::SyncTone;

    fn at(text: &str) -> OffsetDateTime {
        OffsetDateTime::parse(text, &Rfc3339).unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }

    fn status(phase: Phase) -> SyncStatus {
        SyncStatus {
            phase,
            pouch_id: Some("p".into()),
            last_synced_at: Some("2026-10-09T11:55:00Z".into()),
            message: None,
            sent: 0,
            received: 0,
            conflicts: 0,
            conflicts_notified: 0,
            problem_notified: None,
            daemon_pid: Some(1),
            updated_at: "2026-10-09T11:55:00Z".into(),
        }
    }

    #[test]
    fn a_rejected_token_is_told_apart_from_other_failures() {
        let rejected = |code: &str| {
            anyhow::Error::from(SyncError::Api(Box::new(sp_api::ApiError::Server {
                code: code.into(),
                status: 401,
                message: String::new(),
                details: None,
                retryable: false,
            })))
        };
        assert!(token_rejected(&rejected("token_invalid")));
        assert!(token_rejected(
            &rejected("token_invalid").context("syncing")
        ));
        assert!(!token_rejected(&rejected("device_revoked")));
    }

    #[test]
    fn the_sync_line_says_what_is_going_on() {
        let now = at("2026-10-09T12:00:00Z");
        assert_eq!(view(None, false, false, now), None);
        let idle = view(Some(&status(Phase::Idle)), true, true, now);
        assert_eq!(
            idle.as_ref().map(|v| (v.tone, v.text.as_str())),
            Some((SyncTone::Good, "Synced 5 min ago"))
        );
        let stalled = view(Some(&status(Phase::Idle)), false, true, now);
        assert_eq!(stalled.as_ref().map(|v| v.tone), Some(SyncTone::Warn));
        assert!(
            stalled
                .and_then(|v| v.detail)
                .is_some_and(|d| d.contains("daemon start"))
        );
        let offline = view(Some(&status(Phase::Offline)), true, true, now);
        assert_eq!(
            offline.and_then(|v| v.detail).as_deref(),
            Some("Can't reach the server · last synced 5 min ago")
        );
        let mut failed = status(Phase::Failed);
        failed.message = Some("blob_missing\nmore".into());
        let failed = view(Some(&failed), true, true, now);
        assert_eq!(failed.as_ref().map(|v| v.tone), Some(SyncTone::Bad));
        assert_eq!(
            failed.and_then(|v| v.detail).as_deref(),
            Some("blob_missing")
        );
    }

    #[test]
    fn our_copies_and_known_read_only_items_are_not_brought_in() -> anyhow::Result<()> {
        use sp_sync::item::{File, Files};
        let tmp = tempfile::tempdir()?;
        let h = tmp.path();
        let write = |path: &std::path::Path, text: &str| -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap_or(h))?;
            std::fs::write(path, text)
        };
        let copy = h.join(".gemini/commands/ship.toml");
        write(&copy, "prompt = \"Ship {{args}}\"\n")?;
        // OpenCode inside a git checkout, with other rules than the pouch's.
        write(&h.join(".config/opencode/.git/HEAD"), "ref")?;
        write(&h.join(".config/opencode/AGENTS.md"), "git rules")?;
        write(&h.join(".config/opencode/agents/new.md"), "a new agent")?;

        let dir = PouchDir::new(h.join("pouch"));
        let rules: Files = [("INSTRUCTIONS.md".to_string(), File::new("pouch rules"))].into();
        dir.write_item("instructions", "instructions", &rules)?;
        let mut projection = Projection::default();
        projection.copies.insert(
            copy.clone(),
            project::Copied {
                from: h.join("pouch/commands/ship/COMMAND.md"),
                render: project::Render::TomlCommand,
                hash: String::new(),
            },
        );
        let places = Places::plain(h, &h.join(".skillpouch"));
        let harnesses = sp_harness::discover(&places);
        let targets = sp_harness::targets(&places);
        let mut report = Report::default();
        bring_in_new(
            &dir,
            &harnesses,
            &targets,
            &mut projection,
            &h.join("backups"),
            &mut report,
        )?;

        let (items, _) = dir.read()?;
        let mut keys: Vec<_> = items.keys().cloned().collect();
        keys.sort();
        // The new agent comes in; the rules and our own copy don't.
        assert_eq!(keys, ["agent/new", "instructions/instructions"]);
        assert!(copy.exists());
        assert!(h.join(".config/opencode/agents/new.md").exists());

        // Deleted in another assistant: the git checkout's copy stays out.
        write(
            &h.join(".config/opencode/agents/gone.md"),
            "deleted elsewhere",
        )?;
        projection.dismissed.insert("agent/gone".into());
        projection.dismissed.insert("agent/new".into());
        bring_in_new(
            &dir,
            &harnesses,
            &targets,
            &mut projection,
            &h.join("backups"),
            &mut report,
        )?;
        assert!(!dir.read()?.0.contains_key("agent/gone"));
        // `new` is in the pouch again, so it's no longer dismissed.
        assert_eq!(projection.dismissed, ["agent/gone".to_string()].into());
        Ok(())
    }
}
