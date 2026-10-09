//! `skillpouch approve`: hooks and plugins from elsewhere run here only
//! after this.

use crate::approve::{self, label};
use crate::context::Context;
use crate::{daemon, sync, ui};
use anyhow::Result;
use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

pub async fn run(ctx: &Context, names: Vec<String>, yes: bool) -> Result<ExitCode> {
    let waiting = approve::waiting(ctx)?;
    let listed: Vec<_> = waiting
        .iter()
        .filter(|w| names.is_empty() || names.iter().any(|n| n.eq_ignore_ascii_case(&w.name)))
        .collect();
    if waiting.is_empty() {
        ui::success("Nothing is waiting for your OK.");
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "{}",
        ui::heading("These run commands on this computer by themselves:")
    );
    for item in &listed {
        let changed = if item.changed {
            " (changed; the version you approved runs until then)"
        } else {
            ""
        };
        println!("  {}{}", ui::accent(label(item)), ui::dim(changed));
        println!("    {}", item.detail);
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            ui::hint("Run `skillpouch approve --yes` to let them run.");
            return Ok(ExitCode::from(1));
        }
        print!("Let them run here? [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            ui::hint("Nothing approved.");
            return Ok(ExitCode::SUCCESS);
        }
    }
    let names = (!names.is_empty()).then_some(names.as_slice());
    let approved = approve::approve(ctx, names)?;
    if daemon::running(ctx) {
        ui::success(format!(
            "Approved {}. The background sync puts them in place.",
            approved.len()
        ));
        return Ok(ExitCode::SUCCESS);
    }
    let mut session = ctx.session()?;
    sync::run(ctx, &mut session).await?;
    ui::success(format!("Approved {} and put in place.", approved.len()));
    Ok(ExitCode::SUCCESS)
}
