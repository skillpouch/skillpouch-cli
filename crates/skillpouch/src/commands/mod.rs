mod login;
mod logout;
mod status;

use crate::cli::{Cli, Command};
use crate::context::Context;
use anyhow::Result;
use std::process::ExitCode;

pub async fn run(cli: Cli) -> Result<ExitCode> {
    let ctx = Context::load()?;
    match cli.command {
        Command::Login {
            token,
            name,
            no_browser,
        } => login::run(&ctx, token, name, no_browser).await,
        Command::Logout => logout::run(&ctx),
        Command::Status { json } => status::run(&ctx, json).await,
    }
}
