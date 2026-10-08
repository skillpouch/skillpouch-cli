use crate::context::Context;
use crate::ui;
use anyhow::Result;
use sp_core::Credentials;
use std::process::ExitCode;

pub fn run(ctx: &Context) -> Result<ExitCode> {
    let Some(credentials) = forget(ctx)? else {
        ui::hint("Not logged in, nothing to do.");
        return Ok(ExitCode::SUCCESS);
    };
    ui::success("Logged out. This computer's keys and tokens are deleted.");
    ui::hint(format!(
        "It stays on your Devices page until you remove it there: {}",
        ui::accent(format!("{}/app/account/devices", credentials.api_url))
    ));
    Ok(ExitCode::SUCCESS)
}

/// Deletes this computer's keys and tokens; returns them if there were any.
pub fn forget(ctx: &Context) -> Result<Option<Credentials>> {
    let credentials = Credentials::load(&ctx.home)?;
    if credentials.is_some() {
        Credentials::remove(&ctx.home)?;
    }
    Ok(credentials)
}
