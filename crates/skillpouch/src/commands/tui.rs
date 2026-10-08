//! `skillpouch` on its own in an interactive terminal: the home screen. What
//! the user picks there (log in, log out) runs as the normal command once the
//! screen has closed.

use super::{login, logout, status};
use crate::context::{Context, VERSION};
use crate::exit::Exit;
use anyhow::Result;
use sp_tui::{Account, Action, Connection, Launch};
use std::process::ExitCode;
use std::sync::mpsc;
use tokio::runtime::Handle;

pub async fn run(ctx: &Context) -> Result<ExitCode> {
    let launch = Launch {
        version: format!("v{VERSION}"),
        server: ctx.client.base().to_string(),
    };
    let runtime = Handle::current();
    let check = || {
        if !matches!(ctx.credentials(), Ok(None)) {
            let (sender, receiver) = mpsc::channel();
            runtime.spawn(async move {
                let _ = sender.send(check_account().await);
            });
            Some(receiver)
        } else {
            None
        }
    };
    let action = tokio::task::block_in_place(|| sp_tui::run(launch, check))?;
    match action {
        Action::Login => login::run(ctx, None, None, false).await,
        Action::Logout => logout::run(ctx),
        Action::Quit | Action::Refresh => Ok(ExitCode::SUCCESS),
    }
}

async fn check_account() -> Connection {
    match fetch_account().await {
        Ok(account) => Connection::Connected(account),
        Err(error) => Connection::Failed(match error.downcast_ref::<Exit>() {
            Some(exit) => exit.message.clone(),
            None => error.to_string(),
        }),
    }
}

async fn fetch_account() -> Result<Account> {
    let ctx = Context::load()?;
    let mut session = ctx.session()?;
    let me = status::fetch_me(&ctx, &mut session).await?;
    Ok(Account {
        name: me.user.name,
        email: me.user.email,
        computer: me.device.map(|device| device.name),
        plan: me.account.entitlements.plan_name,
        used_bytes: me.account.used_bytes,
        quota_bytes: me.account.quota_bytes,
        delete_after: me.account.delete_after,
    })
}
