//! `skillpouch` on its own outside an interactive terminal (pipes, scripts):
//! the banner, whether this computer is connected, and the commands to try.

use crate::context::{Context, VERSION};
use crate::ui;
use anyhow::Result;
use std::process::ExitCode;

const COMMANDS: &[(&str, &str)] = &[
    ("login", "Connect this computer to your account"),
    ("status", "Account, plan and storage"),
    ("sync", "Sync this computer's pouch now"),
    ("daemon start", "Keep it in sync in the background"),
    ("logout", "Forget this computer's keys"),
];

pub fn run(ctx: &Context) -> Result<ExitCode> {
    ui::banner(&format!("v{VERSION}"));
    if ctx.credentials()?.is_some() {
        ui::success(format!("Connected to {}", ctx.client.base()));
    } else {
        ui::hint(format!(
            "Not connected yet. Run `skillpouch login` to connect to {}.",
            ctx.client.base()
        ));
    }
    println!();
    println!("{}", ui::heading("Commands"));
    ui::commands(COMMANDS);
    println!();
    println!(
        "{} {} {}",
        ui::dim("Run"),
        ui::accent("skillpouch <command> --help"),
        ui::dim("for details.")
    );
    Ok(ExitCode::SUCCESS)
}
