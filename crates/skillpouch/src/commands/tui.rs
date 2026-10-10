//! `skillpouch` on its own in an interactive terminal: the dashboard. The TUI
//! draws; this side answers its requests (load the account, set up and pick
//! the pouch to sync, log in and out, open links) and sends back what
//! happened.

use super::{login, logout, status};
use crate::browser;
use crate::context::{Context, VERSION, signed_out};
use crate::exit::{Exit, NOT_LOGGED_IN};
use crate::usage::Reported;
use crate::{daemon, names, setup, usage, when};
use anyhow::Result;
use sp_api::types::{ArtifactList, DeviceList, Me, PouchList};
use sp_core::setup::SetupRecord;
use sp_core::{Credentials, assistants};
use sp_crypto::vault::PouchMeta;
use sp_tui::usage::Usage;
use sp_tui::{Account, Computer, Dashboard, Launch, LoginStep, Notice, Pouch, Request, Update};
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Items counted per pouch; more show as "200+".
const ITEMS_COUNTED: u32 = 200;

/// How long quitting waits to tell the server an open login was abandoned.
const CANCEL_GRACE: Duration = Duration::from_secs(3);

pub async fn run(ctx: Context) -> Result<ExitCode> {
    let ctx = Arc::new(ctx);
    let launch = Launch {
        version: format!("v{VERSION}"),
        server: ctx.client.base().to_string(),
        hostname: gethostname::gethostname()
            .to_string_lossy()
            .trim()
            .to_string(),
    };
    let signed_in = matches!(ctx.credentials(), Ok(Some(_)));
    let (updates, updates_rx) = std::sync::mpsc::channel();
    let (requests, requests_rx) = mpsc::unbounded_channel();
    let worker = tokio::spawn(serve(Arc::clone(&ctx), requests_rx, updates));
    if signed_in {
        let _ = requests.send(Request::Refresh);
    }

    let shown = tokio::task::block_in_place(|| {
        sp_tui::run(
            launch,
            signed_in,
            |request| {
                let _ = requests.send(request);
            },
            updates_rx,
        )
    });
    drop(requests);
    let _ = tokio::time::timeout(CANCEL_GRACE, worker).await;
    if shown? == sp_tui::Closed::Restart {
        return Ok(crate::update::run_again());
    }
    if crate::update::exe().is_some_and(|exe| crate::update::temporary(&exe)) {
        crate::ui::warn(
            "This is a temporary copy of SkillPouch (from npx), so nothing syncs in the background.",
        );
        crate::ui::hint("Install it to keep this computer in sync: `npm i -g skillpouch`.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Answers the TUI until it closes.
async fn serve(
    ctx: Arc<Context>,
    mut requests: mpsc::UnboundedReceiver<Request>,
    updates: Sender<Update>,
) {
    let mut loading: Option<JoinHandle<()>> = None;
    let mut login_flow: Option<(JoinHandle<()>, oneshot::Sender<()>)> = None;
    let reported = Arc::new(Reported::default());
    // Setup is offered once per run; skipped, it waits for the Account tab.
    let offered = Arc::new(AtomicBool::new(false));
    let starting = Arc::clone(&ctx);
    tokio::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || daemon::ensure_running(&starting)).await;
    });
    let watcher = tokio::spawn(watch_sync(Arc::clone(&ctx), updates.clone()));
    let (updating, told) = (Arc::clone(&ctx), updates.clone());
    tokio::spawn(async move {
        crate::update::in_dashboard(&updating, |step| {
            let _ = told.send(Update::Upgrade(step));
        })
        .await;
    });

    while let Some(request) = requests.recv().await {
        match request {
            Request::Refresh => {
                if loading.as_ref().is_none_or(JoinHandle::is_finished) {
                    let (ctx, updates) = (Arc::clone(&ctx), updates.clone());
                    let (reported, offered) = (Arc::clone(&reported), Arc::clone(&offered));
                    loading = Some(tokio::spawn(async move {
                        load_and_offer(&ctx, &reported, &offered, &updates).await;
                    }));
                }
            }
            Request::Login => {
                if let Some((_, cancel)) = login_flow.take() {
                    let _ = cancel.send(());
                }
                let (cancel, cancelled) = oneshot::channel();
                let flow = tokio::spawn(log_in(
                    Arc::clone(&ctx),
                    Arc::clone(&reported),
                    Arc::clone(&offered),
                    updates.clone(),
                    cancelled,
                ));
                login_flow = Some((flow, cancel));
            }
            Request::CancelLogin => {
                if let Some((_, cancel)) = login_flow.take() {
                    let _ = cancel.send(());
                }
            }
            Request::Logout => {
                let credentials = Credentials::load(&ctx.home).ok().flatten();
                let removed = match credentials {
                    Some(credentials) => logout::revoke(&ctx, credentials).await,
                    None => true,
                };
                let update = match Credentials::remove(&ctx.home) {
                    Ok(_) => {
                        let _ = updates.send(Update::SignedOut);
                        if removed {
                            Notice::success("Logged out. This computer is off your Devices page.")
                        } else {
                            Notice::info(
                                "Logged out here. Couldn't reach the server: remove it on your Devices page.",
                            )
                        }
                    }
                    Err(error) => Notice::error(error.to_string()),
                };
                let _ = updates.send(Update::Notice(update));
            }
            Request::UsePouch(id) => {
                if let Err(error) = use_pouch(&ctx, &id).await {
                    let _ = updates.send(Update::Notice(Notice::error(format!(
                        "Couldn't switch pouches: {}",
                        message(&error)
                    ))));
                }
                // Either way the server has the last word.
                let _ = updates.send(load(&ctx).await);
            }
            Request::Choose { assistant, id, on } => {
                let choice = if assistant {
                    crate::choose::parse(&id)
                        .filter(|c| matches!(c, crate::choose::Choice::Assistant(_)))
                } else {
                    crate::choose::parse(&id)
                };
                let result = match choice {
                    Some(choice) if on => crate::choose::set(&ctx, &[choice], &[]),
                    Some(choice) => crate::choose::set(&ctx, &[], &[choice]),
                    None => Err(anyhow::anyhow!("unknown: {id}")),
                };
                if let Err(error) = result {
                    let _ = updates.send(Update::Notice(Notice::error(message(&error))));
                }
            }
            Request::Setup => {
                let (ctx, updates) = (Arc::clone(&ctx), updates.clone());
                tokio::spawn(async move {
                    let _ = updates.send(Update::Setup(setup::assistants(&ctx).await));
                });
            }
            Request::FinishSetup(choice) => {
                let saved = setup::save(&ctx, choice).await;
                let _ = updates.send(Update::SetupSaved(
                    saved.as_ref().map(|_| ()).map_err(message),
                ));
                if saved.is_ok() {
                    keep_syncing(&ctx).await;
                    let _ = updates.send(load(&ctx).await);
                }
            }
            Request::Open(url) => {
                if !browser::open(&url) {
                    let _ = updates.send(Update::Notice(Notice::error(format!(
                        "Couldn't open a browser. Go to {url}"
                    ))));
                }
            }
        }
    }
    watcher.abort();
    // The TUI closed: an open login is abandoned, not left waiting.
    if let Some((flow, cancel)) = login_flow {
        let _ = cancel.send(());
        let _ = flow.await;
    }
}

async fn log_in(
    ctx: Arc<Context>,
    reported: Arc<Reported>,
    offered: Arc<AtomicBool>,
    updates: Sender<Update>,
    mut cancelled: oneshot::Receiver<()>,
) {
    let send = |step| {
        let _ = updates.send(Update::Login(step));
    };
    let pending = tokio::select! {
        pending = login::begin(&ctx, None, None) => pending,
        _ = &mut cancelled => return,
    };
    let pending = match pending {
        Ok(pending) => pending,
        Err(error) => return send(LoginStep::Failed(message(&error))),
    };
    send(LoginStep::Waiting {
        url: pending.authorize_url.clone(),
        code: pending.code.clone(),
        computer: pending.device_name.clone(),
        opened: browser::open(&pending.authorize_url),
    });
    tokio::select! {
        approved = pending.wait(&ctx) => {
            let saved = match approved {
                Ok(approved) => pending.save(&ctx, approved).await,
                Err(error) => Err(error),
            };
            match saved {
                Ok(()) => {
                    send(LoginStep::Approved);
                    // A new login is set up afresh.
                    offered.store(false, Ordering::SeqCst);
                    load_and_offer(&ctx, &reported, &offered, &updates).await;
                }
                Err(error) => send(LoginStep::Failed(message(&error))),
            }
        }
        _ = &mut cancelled => {
            pending.cancel(&ctx).await;
            send(LoginStep::Cancelled);
        }
    }
}

/// Loads the dashboard, then offers setup if this login hasn't had it.
async fn load_and_offer(
    ctx: &Context,
    reported: &Reported,
    offered: &AtomicBool,
    updates: &Sender<Update>,
) {
    let update = load(ctx).await;
    let loaded = matches!(update, Update::Loaded(_));
    let _ = updates.send(update);
    if loaded && let Ok(session) = ctx.session() {
        let usage = usage::sync(ctx, &session, reported).await;
        let _ = updates.send(Update::Usage(usage));
    }
    let Ok(Some(credentials)) = ctx.credentials() else {
        return;
    };
    if loaded
        && SetupRecord::needed(&ctx.home, &credentials.device_id)
        && !offered.swap(true, Ordering::SeqCst)
    {
        let _ = updates.send(Update::Setup(setup::assistants(ctx).await));
    }
}

async fn use_pouch(ctx: &Context, pouch_id: &str) -> Result<()> {
    let mut session = ctx.session()?;
    let token = session.access_token(ctx, false).await?;
    ctx.client
        .set_active_pouch(&session.key, &token, Some(pouch_id))
        .await
        .map_err(signed_out)?;
    // The background sync notices and switches the links over.
    crate::sync::switch_pouch(ctx, &session.credentials.device_id, pouch_id)
}

/// The sync line for this computer right now.
fn sync_view(ctx: &Context) -> Option<sp_tui::SyncView> {
    let device_id = ctx.credentials().ok().flatten()?.device_id;
    let set_up = !SetupRecord::needed(&ctx.home, &device_id);
    let status = sp_sync::status::SyncStatus::load(&ctx.home);
    crate::sync::view(
        status.as_ref(),
        daemon::running(ctx),
        set_up,
        OffsetDateTime::now_utc(),
    )
}

/// Starts the background sync once this computer is set up.
async fn keep_syncing(ctx: &Arc<Context>) {
    let ctx = Arc::clone(ctx);
    let _ = tokio::task::spawn_blocking(move || {
        let device_id = ctx.credentials().ok().flatten()?.device_id;
        if SetupRecord::needed(&ctx.home, &device_id) {
            return None;
        }
        daemon::start(&ctx).ok()
    })
    .await;
}

/// Sends the sync line whenever it changes, until the TUI closes.
async fn watch_sync(ctx: Arc<Context>, updates: Sender<Update>) {
    let mut shown = None;
    loop {
        let view = {
            let ctx = Arc::clone(&ctx);
            tokio::task::spawn_blocking(move || sync_view(&ctx))
                .await
                .ok()
                .flatten()
        };
        if view != shown {
            if updates.send(Update::Sync(view.clone())).is_err() {
                return;
            }
            shown = view;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn load(ctx: &Context) -> Update {
    match ctx.credentials() {
        Ok(Some(_)) => {}
        Ok(None) => return Update::SignedOut,
        Err(error) => return Update::Failed(message(&error)),
    }
    match fetch(ctx).await {
        Ok(dashboard) => Update::Loaded(Box::new(dashboard)),
        // Removed or expired: this computer has to log in again.
        Err(error)
            if error
                .downcast_ref::<Exit>()
                .is_some_and(|exit| exit.code == NOT_LOGGED_IN) =>
        {
            Update::SignedOut
        }
        Err(error) => Update::Failed(message(&error)),
    }
}

async fn fetch(ctx: &Context) -> Result<Dashboard> {
    let mut session = ctx.session()?;
    // Refreshes the access token first when it needs it.
    let me = status::fetch_me(ctx, &mut session).await?;
    let token = session.credentials.access_token.as_str();
    let key = &session.key;
    // Names are extras: without them the dashboard still shows. Usage comes
    // later (counting the logs can take a while).
    let (listed, names) = tokio::join!(
        async {
            tokio::try_join!(
                ctx.client.pouches(key, token),
                ctx.client.devices(key, token),
            )
        },
        names::pouch_names(ctx, &session),
    );
    let (pouches, devices) = listed.map_err(signed_out)?;
    let active = devices
        .devices
        .iter()
        .find(|device| device.is_current)
        .and_then(|device| device.active_pouch_id.as_deref());
    let first_sync = setup::pending(ctx, &session.credentials.device_id, active);
    let mut contents = Vec::new();
    for pouch in pouches.pouches.iter().filter(|p| p.state != "deleted") {
        let items = ctx
            .client
            .artifacts(key, token, &pouch.id, ITEMS_COUNTED)
            .await
            .map_err(signed_out)?;
        contents.push((pouch.id.clone(), items));
    }
    let mut dashboard = dashboard(
        Answers {
            me,
            pouches,
            devices,
            contents,
            names,
            usage: None,
            first_sync,
        },
        OffsetDateTime::now_utc(),
    );
    dashboard.sync = sync_view(ctx);
    dashboard.choice = choice_view(ctx);
    Ok(dashboard)
}

/// What the server said, and what this computer could decrypt.
struct Answers {
    me: Me,
    pouches: PouchList,
    devices: DeviceList,
    contents: Vec<(String, ArtifactList)>,
    names: Result<BTreeMap<String, PouchMeta>, names::Missing>,
    usage: Option<Result<Usage, String>>,
    /// The first sync chosen at setup, while it hasn't run.
    first_sync: Option<String>,
}

fn dashboard(answers: Answers, now: OffsetDateTime) -> Dashboard {
    let Answers {
        me,
        pouches,
        devices,
        contents,
        names,
        usage,
        first_sync,
    } = answers;
    let (names, names_missing) = match names {
        Ok(names) => (names, None),
        Err(missing) => (BTreeMap::new(), Some(missing)),
    };
    let mut listed = pouches.pouches;
    listed.sort_by_key(|pouch| pouch.created_seq);
    let pouches = listed
        .into_iter()
        .enumerate()
        .map(|(index, pouch)| {
            let items = contents
                .iter()
                .find(|(id, _)| id == &pouch.id)
                .map_or(&[][..], |(_, list)| list.items.as_slice());
            let updated = items.iter().map(|a| a.updated_at.as_str()).max();
            let meta = names.get(&pouch.id);
            Pouch {
                number: index + 1,
                name: meta.map(|meta| meta.name.clone()),
                description: meta
                    .map(|meta| meta.description.clone())
                    .filter(|d| !d.is_empty()),
                state: pouch.state,
                items: items.len(),
                more: items.len() >= ITEMS_COUNTED as usize,
                updated: updated.and_then(|at| when::ago(at, now)),
                event_seq: pouch.event_seq,
                id: pouch.id,
            }
        })
        .collect();
    let computers = devices
        .devices
        .into_iter()
        .filter(|device| device.state != "revoked")
        .map(|device| Computer {
            platform: format!("{} {}", os_name(&device.os), device.arch),
            assistants: device
                .assistants
                .iter()
                .map(|id| assistants::name_of(id).unwrap_or(id).to_string())
                .collect(),
            last_seen: device
                .last_seen_at
                .as_deref()
                .and_then(|at| when::ago(at, now)),
            connected: when::date(&device.created_at, now),
            cursors: device
                .cursors
                .into_iter()
                .map(|cursor| (cursor.pouch_id, cursor.acked_seq))
                .collect(),
            name: device.name,
            cli_version: device.cli_version,
            key_storage: device.key_storage,
            state: device.state,
            current: device.is_current,
            active_pouch: device.active_pouch_id,
        })
        .collect();
    Dashboard {
        account: Account {
            name: me.user.name,
            email: me.user.email,
            plan: me.account.entitlements.plan_name,
            pouch_limit: me.account.entitlements.pouch_limit,
            used_bytes: me.account.used_bytes,
            quota_bytes: me.account.quota_bytes,
            delete_after: me
                .account
                .delete_after
                .as_deref()
                .and_then(|at| when::date(at, now)),
        },
        pouches,
        computers,
        browsers: devices
            .browsers
            .iter()
            .filter(|browser| browser.revoked_at.is_none())
            .count(),
        names_missing,
        usage,
        first_sync,
        sync: None,
        choice: sp_tui::ChoiceView::default(),
    }
}

/// The Sync tab: the assistants found here and the kinds, on or off.
fn choice_view(ctx: &Context) -> sp_tui::ChoiceView {
    let choice = crate::choose::current(ctx);
    let targets = crate::sync::places(ctx)
        .map(|places| sp_harness::targets(&places))
        .unwrap_or_default();
    sp_tui::ChoiceView {
        assistants: targets
            .iter()
            .map(|target| sp_tui::ChoiceRow {
                id: target.id.to_string(),
                name: target.name.to_string(),
                on: choice.assistant_on(target.id),
                note: target
                    .read_only
                    .as_ref()
                    .map(|_| "only read: part of a git checkout".to_string()),
            })
            .collect(),
        kinds: crate::choose::KINDS
            .iter()
            .map(|(kind, label)| sp_tui::ChoiceRow {
                id: kind.to_string(),
                name: label.to_string(),
                on: choice.kind_on(kind),
                note: None,
            })
            .collect(),
    }
}

fn os_name(os: &str) -> &str {
    match os {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn message(error: &anyhow::Error) -> String {
    match error.downcast_ref::<Exit>() {
        Some(exit) => exit.message.clone(),
        None => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, from_value, json};
    use time::format_description::well_known::Rfc3339;

    fn parse<T: serde::de::DeserializeOwned>(value: Value) -> T {
        match from_value(value) {
            Ok(parsed) => parsed,
            Err(error) => panic!("bad fixture: {error}"),
        }
    }

    #[test]
    fn builds_the_dashboard_from_the_server_answers() {
        let now = OffsetDateTime::parse("2026-10-09T12:00:00Z", &Rfc3339)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let me: Me = parse(json!({
            "user": { "email": "ada@example.com", "name": "Ada" },
            "account": {
                "id": "a", "quota_bytes": 100, "used_bytes": 10, "delete_after": null,
                "entitlements": { "plan_slug": "free", "plan_name": "Free", "pouch_limit": 1 }
            },
            "device": null
        }));
        let pouches: PouchList = parse(json!({ "pouches": [
            { "id": "p2", "state": "active", "event_seq": 9, "key_gen": 1, "created_seq": 5,
              "epoch": "e", "delete_after": null, "created_at": "2026-10-01T00:00:00Z" },
            { "id": "p1", "state": "locked", "event_seq": 2, "key_gen": 1, "created_seq": 1,
              "epoch": "e", "delete_after": null, "created_at": "2026-09-01T00:00:00Z" }
        ]}));
        let devices: DeviceList = parse(json!({
            "devices": [
                { "id": "d1", "name": "laptop", "os": "macos", "arch": "aarch64",
                  "cli_version": "0.1.0", "assistants": ["claude-code", "mystery"],
                  "cursors": [{ "pouch_id": "p2", "acked_seq": 9 }], "state": "active",
                  "key_storage": "file", "member_id": null, "active_pouch_id": "p2",
                  "last_seen_at": "2026-10-09T11:58:00Z", "revoked_at": null,
                  "created_at": "2026-10-08T00:00:00Z", "is_current": true },
                { "id": "d2", "name": "old", "os": "linux", "arch": "x86_64",
                  "cli_version": "0.1.0", "assistants": [], "cursors": [], "state": "revoked",
                  "key_storage": null, "member_id": null, "active_pouch_id": null,
                  "last_seen_at": null, "revoked_at": "2026-10-01T00:00:00Z",
                  "created_at": "2026-09-01T00:00:00Z", "is_current": false }
            ],
            "browsers": [
                { "member_id": "m", "label_enc": null, "last_seen_at": null,
                  "revoked_at": null, "removed_seq": null, "is_current": false },
                { "member_id": "n", "label_enc": null, "last_seen_at": null,
                  "revoked_at": "2026-10-01T00:00:00Z", "removed_seq": 3, "is_current": false }
            ]
        }));
        let items: ArtifactList = parse(json!({ "items": [
            { "id": "x", "name_index": "n", "deleted_at": null, "head": null,
              "updated_at": "2026-10-09T09:00:00Z" },
            { "id": "y", "name_index": "m", "deleted_at": null, "head": null,
              "updated_at": "2026-10-09T08:00:00Z" }
        ]}));

        let names = BTreeMap::from([(
            "p2".to_string(),
            PouchMeta {
                name: "Work stuff".into(),
                description: String::new(),
            },
        )]);
        let dashboard = dashboard(
            Answers {
                me,
                pouches,
                devices,
                contents: vec![("p2".into(), items)],
                names: Ok(names),
                usage: None,
                first_sync: Some("Merge with Claude Code".into()),
            },
            now,
        );
        let numbers: Vec<_> = dashboard
            .pouches
            .iter()
            .map(|p| (p.id.as_str(), p.number, p.items))
            .collect();
        assert_eq!(numbers, [("p1", 1, 0), ("p2", 2, 2)]);
        let labels: Vec<_> = dashboard.pouches.iter().map(Pouch::label).collect();
        assert_eq!(labels, ["Pouch 1", "Work stuff"]);
        assert_eq!(dashboard.pouches[1].description, None);
        assert_eq!(dashboard.names_missing, None);
        assert_eq!(dashboard.pouches[1].updated.as_deref(), Some("3 h ago"));
        assert_eq!(dashboard.computers.len(), 1);
        let laptop = &dashboard.computers[0];
        assert_eq!(laptop.platform, "macOS aarch64");
        assert_eq!(laptop.assistants, ["Claude Code", "mystery"]);
        assert_eq!(laptop.last_seen.as_deref(), Some("2 min ago"));
        assert_eq!(laptop.connected.as_deref(), Some("Oct 8"));
        assert_eq!(dashboard.browsers, 1);
        assert_eq!(dashboard.account.plan, "Free");
        assert_eq!(
            dashboard.first_sync.as_deref(),
            Some("Merge with Claude Code")
        );
    }
}
