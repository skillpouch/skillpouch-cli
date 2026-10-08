use crate::context::Context;
use anyhow::Result;
use sp_core::Credentials;
use std::process::ExitCode;

pub fn run(ctx: &Context) -> Result<ExitCode> {
    let Some(credentials) = Credentials::load(&ctx.home)? else {
        println!("Not logged in.");
        return Ok(ExitCode::SUCCESS);
    };
    Credentials::remove(&ctx.home)?;
    println!("Logged out: this computer's keys and tokens are deleted.");
    println!(
        "It still appears on your Devices page until you remove it there:\n  {}/app/account/devices",
        credentials.api_url
    );
    Ok(ExitCode::SUCCESS)
}
