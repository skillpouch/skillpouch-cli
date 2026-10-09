mod home;
mod login;
mod logout;
mod status;
mod sync;
mod tui;

use crate::cli::{Cli, Command, DaemonAction};
use crate::context::Context;
use anyhow::Result;
use std::io::IsTerminal;
use std::process::ExitCode;

pub async fn run(cli: Cli) -> Result<ExitCode> {
    let ctx = Context::load()?;
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
        Command::Logout => logout::run(&ctx).await,
        Command::Status { json } => status::run(&ctx, json).await,
        Command::Sync => sync::once(&ctx).await,
        Command::Daemon { action } => match action {
            DaemonAction::Start => sync::start(&ctx),
            DaemonAction::Stop => sync::stop(&ctx),
            DaemonAction::Status => sync::status(&ctx),
            DaemonAction::Run => sync::run_daemon(&ctx).await,
        },
    }
}
