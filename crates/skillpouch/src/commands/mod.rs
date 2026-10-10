mod home;
mod login;
mod logout;
mod select;
mod status;
mod sync;
mod tui;

use crate::cli::{Cli, Command, DaemonAction};
use crate::context::Context;
use anyhow::Result;
use std::io::IsTerminal;
use std::process::ExitCode;

/// Someone is at the terminal (not a pipe or a script).
pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

pub async fn run(cli: Cli) -> Result<ExitCode> {
    let ctx = match &cli.command {
        Some(Command::Daemon {
            action: DaemonAction::Run { home, api_url, .. },
        }) => daemon_context(home.clone(), api_url.clone())?,
        _ => Context::load()?,
    };
    if updates(cli.command.as_ref())
        && interactive()
        && let Some(code) = crate::update::before_command(&ctx).await
    {
        return Ok(code);
    }
    // Syncing runs on its own: whatever is run, a set-up computer whose
    // background sync stopped (a restart without a service) gets it back.
    let heals = !matches!(
        cli.command,
        Some(Command::Daemon { .. } | Command::Logout | Command::Unlink { .. })
    );
    if heals {
        crate::daemon::ensure_running(&ctx);
    }
    let Some(command) = cli.command else {
        if interactive() {
            return tui::run(ctx).await;
        }
        return home::run(&ctx);
    };
    match command {
        Command::Login {
            token,
            name,
            no_browser,
        } => {
            let code = login::run(&ctx, token, name, no_browser).await?;
            if code == ExitCode::SUCCESS {
                // It syncs for the login just replaced; setting up starts a new one.
                let _ = crate::daemon::stop(&ctx);
            }
            // Straight on to setting this computer up; then it runs on its own.
            if code == ExitCode::SUCCESS && interactive() {
                return tui::run(Context::load()?).await;
            }
            Ok(code)
        }
        Command::Select { on, off } => select::select(&ctx, &on, &off),
        Command::Unlink { assistant } => select::unlink(&ctx, assistant.as_deref()),
        Command::Conflicts => sync::conflicts(&ctx),
        Command::Logout => logout::run(&ctx).await,
        Command::Status { json } => status::run(&ctx, json).await,
        Command::Sync => sync::once(&ctx).await,
        Command::Daemon { action } => match action {
            DaemonAction::Start => sync::start(&ctx),
            DaemonAction::Stop => sync::stop(&ctx),
            DaemonAction::Status => sync::status(&ctx),
            DaemonAction::Run {
                background,
                api_url,
                ..
            } => sync::run_daemon(&ctx, background, api_url.is_some()).await,
        },
    }
}

/// Commands that update SkillPouch first when a newer release is out. The
/// dashboard (no command) opens at once and updates behind it.
fn updates(command: Option<&Command>) -> bool {
    !matches!(
        command,
        None | Some(Command::Daemon { .. } | Command::Logout | Command::Unlink { .. })
    ) && !matches!(command, Some(Command::Status { json: true }))
}

/// The background sync talks to the server this computer logged in to,
/// unless told otherwise: a service set up for one server keeps working
/// after logging in to another.
fn daemon_context(home: Option<std::path::PathBuf>, api_url: Option<String>) -> Result<Context> {
    let ctx = Context::load_with(home.clone(), api_url.clone())?;
    if api_url.is_some() {
        return Ok(ctx);
    }
    match crate::daemon::server(&ctx.home) {
        Ok(url) if url != ctx.client.base() => Context::load_with(home, Some(url)),
        _ => Ok(ctx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dashboard_doesnt_wait_for_an_update_but_commands_do() {
        assert!(!updates(None));
        assert!(updates(Some(&Command::Sync)));
        assert!(updates(Some(&Command::Status { json: false })));
        assert!(!updates(Some(&Command::Status { json: true })));
        assert!(!updates(Some(&Command::Logout)));
    }
}
