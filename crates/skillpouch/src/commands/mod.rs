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

pub async fn run(cli: Cli) -> Result<ExitCode> {
    let ctx = match &cli.command {
        Some(Command::Daemon {
            action: DaemonAction::Run { home, api_url, .. },
        }) => Context::load_with(home.clone(), api_url.clone())?,
        _ => Context::load()?,
    };
    let Some(command) = cli.command else {
        if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
            return tui::run(ctx).await;
        }
        return home::run(&ctx);
    };
    match command {
        Command::Login {
            token,
            name,
            no_browser,
        } => login::run(&ctx, token, name, no_browser).await,
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
            DaemonAction::Run { background, .. } => sync::run_daemon(&ctx, background).await,
        },
    }
}
