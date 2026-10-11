//! What this computer syncs (`skillpouch select`, the TUI's Sync tab) and
//! unlinking (`skillpouch unlink`). Turning an assistant or a kind off hands
//! its items back to the assistants right away: links become real files, so
//! nothing stops working.

use crate::context::Context;
use anyhow::{Result, bail};
use sp_core::home::lock_exclusive;
use sp_core::setup::SetupRecord;
use sp_core::{Config, SyncChoice};
use sp_sync::project::Projection;
use sp_sync::release::Released;
use sp_sync::status::Phase;

/// The kinds, as the pouch names them and as people read them.
pub const KINDS: [(&str, &str); 8] = sp_harness::POUCH_KINDS;

/// An assistant or a kind, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Assistant(&'static str),
    Kind(&'static str),
}

/// `codex`, `skills`, `mcp-servers`, `MCP servers`…
pub fn parse(text: &str) -> Option<Choice> {
    let wanted = text.trim().to_lowercase().replace([' ', '_'], "-");
    if let Some((id, _)) = sp_harness::known()
        .into_iter()
        .find(|(id, name)| *id == wanted || name.to_lowercase().replace(' ', "-") == wanted)
    {
        return Some(Choice::Assistant(id));
    }
    let singular = wanted.trim_end_matches('s');
    KINDS
        .iter()
        .find(|(kind, label)| {
            let kind = kind.replace('_', "-");
            kind == wanted
                || kind == singular
                || label.to_lowercase().replace(' ', "-") == wanted
                || (singular == "mcp" && kind == "mcp-server")
                || (wanted == "settings" && kind == "setting")
        })
        .map(|(kind, _)| Choice::Kind(kind))
}

/// The name people read.
pub fn label(choice: Choice) -> String {
    match choice {
        Choice::Assistant(id) => sp_harness::known()
            .into_iter()
            .find(|(known, _)| *known == id)
            .map_or(id, |(_, name)| name)
            .to_string(),
        Choice::Kind(kind) => KINDS
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(kind, |(_, label)| label)
            .to_string(),
    }
}

/// This computer's choice, read fresh (another process may have changed it).
pub fn current(ctx: &Context) -> SyncChoice {
    Config::load(&ctx.home)
        .map(|config| config.sync)
        .unwrap_or_default()
}

/// Turns things on and off, then hands back to the assistants what is off
/// now. What is turned on is linked again by the next sync.
pub fn set(ctx: &Context, on: &[Choice], off: &[Choice]) -> Result<Released> {
    let _lock = lock_exclusive(&sync_lock(ctx))?;
    let mut config = Config::load(&ctx.home)?;
    for choice in on {
        match choice {
            Choice::Assistant(id) => config.sync.skip_assistants.remove(*id),
            Choice::Kind(kind) => config.sync.skip_kinds.remove(*kind),
        };
    }
    for choice in off {
        match choice {
            Choice::Assistant(id) => config.sync.skip_assistants.insert(id.to_string()),
            Choice::Kind(kind) => config.sync.skip_kinds.insert(kind.to_string()),
        };
    }
    if config.sync.skip_kinds.len() == KINDS.len() {
        bail!("Leave at least one kind on, or unlink this computer: `skillpouch unlink`.");
    }
    config.save(&ctx.home)?;
    let targets = sp_harness::targets(&crate::sync::places(ctx)?);
    let mut projection = Projection::load(&ctx.home);
    let released =
        crate::sync::release_unchosen(&ctx.home, &mut projection, &targets, &config.sync)?;
    projection.save(&ctx.home)?;
    Ok(released)
}

/// Stops syncing this computer: the background sync stops, every item is
/// handed back to the assistants, and the setup is forgotten. The pouch
/// stays on the server and in `~/.skillpouch`.
pub fn unlink_all(ctx: &Context) -> Result<Released> {
    let _ = crate::daemon::stop(ctx);
    let _lock = lock_exclusive(&sync_lock(ctx))?;
    let targets = sp_harness::targets(&crate::sync::places(ctx)?);
    let mut projection = Projection::load(&ctx.home);
    let released = sp_sync::release::release(&ctx.home, &mut projection, &targets, &|_, _| true)?;
    projection.save(&ctx.home)?;
    SetupRecord::remove(&ctx.home)?;
    crate::sync::mark(
        ctx,
        Phase::Stopped,
        None,
        Some("This computer was unlinked. Run `skillpouch` to set it up again.".into()),
    );
    Ok(released)
}

/// Held while syncing, choosing or unlinking, so they never overlap.
pub fn sync_lock(ctx: &Context) -> std::path::PathBuf {
    ctx.home.run_dir().join("sync.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_assistants_and_kinds_however_they_are_written() {
        assert_eq!(parse("codex"), Some(Choice::Assistant("codex")));
        assert_eq!(parse("Claude Code"), Some(Choice::Assistant("claude-code")));
        assert_eq!(parse("skills"), Some(Choice::Kind("skill")));
        assert_eq!(parse("hook"), Some(Choice::Kind("hook")));
        assert_eq!(parse("mcp"), Some(Choice::Kind("mcp_server")));
        assert_eq!(parse("MCP servers"), Some(Choice::Kind("mcp_server")));
        assert_eq!(parse("mcp_server"), Some(Choice::Kind("mcp_server")));
        assert_eq!(parse("settings"), Some(Choice::Kind("setting")));
        assert_eq!(label(Choice::Kind("setting")), "Settings");
        assert_eq!(parse("nope"), None);
        assert_eq!(label(Choice::Kind("mcp_server")), "MCP servers");
        assert_eq!(label(Choice::Assistant("gemini-cli")), "Gemini CLI");
    }
}
