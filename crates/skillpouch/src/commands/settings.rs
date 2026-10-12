use crate::cli::SettingsAction;
use crate::context::Context;
use anyhow::{Result, bail};
use sp_core::home::lock_exclusive;
use sp_core::setup::SetupRecord;
use sp_sync::item::PouchDir;
use sp_sync::settings::{DeviceState, Setting};
use std::collections::BTreeSet;
use std::process::ExitCode;

pub fn run(ctx: &Context, action: SettingsAction) -> Result<ExitCode> {
    let _lock = lock_exclusive(&crate::choose::sync_lock(ctx))?;
    let Some(record) = SetupRecord::load(&ctx.home)? else {
        bail!("Connect this computer first with `skillpouch login`.");
    };
    let targets = sp_harness::targets(&crate::sync::places(ctx)?);
    let mut state = DeviceState::load(&ctx.home, &record.pouch_id)?;
    let dir = PouchDir::new(crate::sync::pouch_dir(ctx, &record.pouch_id));
    let (items, _) = dir.read()?;
    let exclude = matches!(&action, SettingsAction::Exclude { .. });
    match action {
        SettingsAction::List { harness } => {
            if let Some(id) = &harness
                && !targets.iter().any(|target| target.id == id)
            {
                bail!("{id} is not installed.");
            }
            for target in &targets {
                if harness.as_deref().is_some_and(|id| id != target.id)
                    || sp_harness::settings(target.id).is_empty()
                {
                    continue;
                }
                println!("{}", target.name);
                let item = items.get(&format!("setting/{}", target.id));
                let entries = item.map(Setting::from_item).transpose()?;
                let baseline = state.harnesses.get(target.id);
                let keys: BTreeSet<_> = entries
                    .iter()
                    .flat_map(|item| item.entries().into_keys())
                    .chain(entries.iter().flat_map(|item| {
                        sp_sync::settings::local_entries(target.id, &target.settings, item)
                    }))
                    .chain(
                        baseline
                            .into_iter()
                            .flat_map(|state| state.excluded.iter().cloned()),
                    )
                    .collect();
                for key in keys {
                    let status = if baseline.is_some_and(|state| state.excluded.contains(&key)) {
                        "excluded on this computer"
                    } else if entries.as_ref().is_some_and(|setting| {
                        sp_sync::settings::translated_on(target.id, &target.settings, setting, &key)
                    }) {
                        "translated"
                    } else if baseline.is_some_and(|state| state.baseline.contains_key(&key)) {
                        "synced"
                    } else {
                        "only in pouch"
                    };
                    println!("  {key}  {status}");
                }
            }
        }
        SettingsAction::Exclude { harness, entry } | SettingsAction::Include { harness, entry } => {
            if !targets
                .iter()
                .any(|target| target.id == harness && !sp_harness::settings(target.id).is_empty())
            {
                bail!("{harness} is not an installed assistant with settings sync.");
            }
            let item = items
                .get(&format!("setting/{harness}"))
                .map(Setting::from_item)
                .transpose()?;
            if !item.as_ref().is_some_and(|item| {
                item.entries().contains_key(&entry)
                    || targets
                        .iter()
                        .find(|target| target.id == harness)
                        .is_some_and(|target| {
                            sp_sync::settings::local_entries(&harness, &target.settings, item)
                                .contains(&entry)
                        })
            }) {
                bail!("{entry} is not in {harness}'s pouch settings.");
            }
            let root = targets
                .iter()
                .find(|target| target.id == harness)
                .map(|target| target.settings.clone())
                .ok_or_else(|| anyhow::anyhow!("assistant is not installed"))?;
            let device = state.harnesses.entry(harness.clone()).or_default();
            if exclude {
                sp_sync::settings::exclude(&root, &harness, &entry, device);
            } else {
                let item = items
                    .get(&format!("setting/{harness}"))
                    .ok_or_else(|| anyhow::anyhow!("settings item is missing"))?;
                sp_sync::settings::include(
                    &root,
                    &harness,
                    &entry,
                    item,
                    &ctx.home.root().join("backups"),
                    device,
                )?;
            }
            state.save(&ctx.home, &record.pouch_id)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
