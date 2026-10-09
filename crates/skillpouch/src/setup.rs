//! Setting this computer up: what its assistants hold, and saving the pouch
//! it syncs with how the first sync starts.

use crate::context::{Context, signed_out};
use anyhow::{Context as _, Result};
use sp_core::assistants;
use sp_core::home::user_home;
use sp_core::setup::{FirstSync, SetupRecord};
use sp_harness::{Harness, Places};
use sp_tui::SetupChoice;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The assistants on this computer and what they hold. Reads files, so it
/// runs off the async threads.
pub async fn assistants(ctx: &Context) -> Vec<Harness> {
    let Some(home) = user_home() else {
        return Vec::new();
    };
    let places = Places::of(&home, ctx.home.root());
    tokio::task::spawn_blocking(move || sp_harness::discover(&places))
        .await
        .unwrap_or_default()
}

/// Makes the chosen pouch this computer's, then remembers how its first
/// sync starts.
pub async fn save(ctx: &Context, choice: SetupChoice) -> Result<()> {
    let mut session = ctx.session()?;
    let token = session.access_token(ctx, false).await?;
    ctx.client
        .set_active_pouch(&session.key, &token, Some(&choice.pouch_id))
        .await
        .map_err(signed_out)?;
    let mut config = sp_core::Config::load(&ctx.home)?;
    config.sync = sp_core::SyncChoice {
        skip_assistants: choice.skip_assistants.iter().cloned().collect(),
        skip_kinds: choice.skip_kinds.iter().cloned().collect(),
    };
    config.save(&ctx.home)?;
    record(
        &session.credentials.device_id,
        choice,
        OffsetDateTime::now_utc(),
    )
    .save(&ctx.home)
    .context("couldn't save this computer's setup")
}

fn record(device_id: &str, choice: SetupChoice, now: OffsetDateTime) -> SetupRecord {
    SetupRecord {
        format: SetupRecord::FORMAT,
        device_id: device_id.to_string(),
        pouch_id: choice.pouch_id,
        first_sync: match choice.first_sync {
            sp_tui::FirstSync::Fill(assistant) => FirstSync::Fill { assistant },
            sp_tui::FirstSync::UsePouch => FirstSync::UsePouch,
            sp_tui::FirstSync::Merge(assistant) => FirstSync::Merge { assistant },
            sp_tui::FirstSync::UseComputer(assistant) => FirstSync::UseComputer { assistant },
        },
        chosen_at: now.format(&Rfc3339).unwrap_or_default(),
        done: false,
    }
}

/// The first sync still waiting for `pouch_id`, in words.
pub fn pending(ctx: &Context, device_id: &str, pouch_id: Option<&str>) -> Option<String> {
    let record = SetupRecord::load(&ctx.home).ok().flatten()?;
    let current = record.device_id == device_id && Some(record.pouch_id.as_str()) == pouch_id;
    (current && !record.done).then(|| describe(&record.first_sync))
}

fn describe(first_sync: &FirstSync) -> String {
    let name = |id: &str| assistants::name_of(id).unwrap_or(id).to_string();
    match first_sync {
        FirstSync::Fill {
            assistant: Some(id),
        } => format!("Start from {}", name(id)),
        FirstSync::Fill { assistant: None } => "Start empty".into(),
        FirstSync::UsePouch => "Use the pouch".into(),
        FirstSync::Merge { assistant } => format!("Merge with {}", name(assistant)),
        FirstSync::UseComputer { assistant } => {
            format!("Replace the pouch with {}", name(assistant))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_the_choice_for_this_login() {
        let at = OffsetDateTime::UNIX_EPOCH;
        let saved = record(
            "d1",
            SetupChoice {
                pouch_id: "p1".into(),
                first_sync: sp_tui::FirstSync::Merge("claude-code".into()),
                skip_assistants: Vec::new(),
                skip_kinds: Vec::new(),
            },
            at,
        );
        assert_eq!(saved.device_id, "d1");
        assert_eq!(saved.chosen_at, "1970-01-01T00:00:00Z");
        assert!(!saved.done);
        assert_eq!(describe(&saved.first_sync), "Merge with Claude Code");
        assert_eq!(
            describe(&FirstSync::Fill { assistant: None }),
            "Start empty"
        );
    }
}
