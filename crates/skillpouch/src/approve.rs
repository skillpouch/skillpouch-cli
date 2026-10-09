//! Letting hooks and plugins from elsewhere run on this computer.

use crate::context::Context;
use crate::sync;
use anyhow::{Result, bail};
use sp_core::setup::SetupRecord;
use sp_sync::approvals::{Approvals, Waiting};
use sp_sync::item::PouchDir;

/// What waits for approval in this computer's pouch.
pub fn waiting(ctx: &Context) -> Result<Vec<Waiting>> {
    let Some(dir) = pouch(ctx)? else {
        return Ok(Vec::new());
    };
    let (items, _) = dir.read()?;
    Ok(Approvals::load(&ctx.home).waiting(items.values()))
}

/// Approves the waiting items named (all of them with `None`); returns
/// what was approved. The background sync then puts them in place.
pub fn approve(ctx: &Context, names: Option<&[String]>) -> Result<Vec<Waiting>> {
    let Some(dir) = pouch(ctx)? else {
        bail!("This computer isn't set up yet: run `skillpouch` and choose a pouch.");
    };
    let (items, _) = dir.read()?;
    let mut approvals = Approvals::load(&ctx.home);
    let waiting = approvals.waiting(items.values());
    if let Some(names) = names {
        for name in names {
            if !waiting.iter().any(|w| w.name.eq_ignore_ascii_case(name)) {
                bail!("Nothing called `{name}` is waiting for approval.");
            }
        }
    }
    let chosen: Vec<Waiting> = waiting
        .into_iter()
        .filter(|w| names.is_none_or(|names| names.iter().any(|n| n.eq_ignore_ascii_case(&w.name))))
        .collect();
    for item in items.values() {
        if chosen
            .iter()
            .any(|w| w.kind == item.kind && w.name == item.name)
        {
            approvals.approve(item);
        }
    }
    approvals.save(&ctx.home)?;
    let mut status = sp_sync::status::SyncStatus::load(&ctx.home);
    if let Some(status) = &mut status {
        status.waiting = approvals.waiting(items.values());
        let _ = status.save(&ctx.home);
    }
    Ok(chosen)
}

fn pouch(ctx: &Context) -> Result<Option<PouchDir>> {
    let Some(credentials) = ctx.credentials()? else {
        return Ok(None);
    };
    Ok(SetupRecord::load(&ctx.home)?
        .filter(|record| record.device_id == credentials.device_id)
        .map(|record| PouchDir::new(sync::pouch_dir(ctx, &record.pouch_id))))
}

/// `Hook format`, as listed.
pub fn label(waiting: &Waiting) -> String {
    let kind = if waiting.kind == "hook" {
        "Hook"
    } else {
        "Plugin"
    };
    format!("{kind} {}", waiting.name)
}
