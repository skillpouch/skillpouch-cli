//! `skillpouch select` (what this computer syncs) and `skillpouch unlink`.

use crate::choose::{self, Choice, KINDS};
use crate::context::Context;
use crate::exit::Exit;
use crate::ui;
use anyhow::Result;
use sp_sync::release::Released;
use std::process::ExitCode;

pub fn select(ctx: &Context, on: &[String], off: &[String]) -> Result<ExitCode> {
    if on.is_empty() && off.is_empty() {
        list(ctx)?;
        return Ok(ExitCode::SUCCESS);
    }
    let on = parse_all(on)?;
    let off = parse_all(off)?;
    let released = choose::set(ctx, &on, &off)?;
    for choice in &on {
        ui::success(format!("{} on.", choose::label(*choice)));
    }
    for choice in &off {
        ui::success(format!("{} off.", choose::label(*choice)));
    }
    handed_back(&released);
    if !on.is_empty() {
        next_sync(ctx);
    }
    Ok(ExitCode::SUCCESS)
}

pub fn unlink(ctx: &Context, assistant: Option<&str>) -> Result<ExitCode> {
    let Some(name) = assistant else {
        let released = choose::unlink_all(ctx)?;
        ui::success("This computer is unlinked: SkillPouch no longer syncs it.");
        handed_back(&released);
        ui::hint(
            "Your pouch is still on the server. Run `skillpouch` to set this computer up again.",
        );
        return Ok(ExitCode::SUCCESS);
    };
    let choice = match choose::parse(name) {
        Some(choice @ Choice::Assistant(_)) => choice,
        _ => return Err(unknown(name, true)),
    };
    let released = choose::set(ctx, &[], &[choice])?;
    ui::success(format!("{} is unlinked.", choose::label(choice)));
    handed_back(&released);
    ui::hint(format!(
        "Link it again with `skillpouch select --on {name}`."
    ));
    Ok(ExitCode::SUCCESS)
}

fn list(ctx: &Context) -> Result<()> {
    let choice = choose::current(ctx);
    let found = sp_harness::targets(&crate::sync::places(ctx)?);
    let mark = |on: bool| {
        if on {
            ui::accent("● on ").to_string()
        } else {
            ui::dim("○ off").to_string()
        }
    };
    println!("{}", ui::heading("Assistants on this computer"));
    if found.is_empty() {
        println!("  {}", ui::dim("none found"));
    }
    for target in &found {
        let note = target
            .read_only
            .as_ref()
            .map(|_| ui::dim("  (only read: part of a git checkout)").to_string())
            .unwrap_or_default();
        println!(
            "  {}  {}{note}",
            mark(choice.assistant_on(target.id)),
            target.name
        );
    }
    println!();
    println!("{}", ui::heading("What goes into them"));
    for (kind, label) in KINDS {
        println!("  {}  {label}", mark(choice.kind_on(kind)));
    }
    println!();
    ui::hint("Change it with `skillpouch select --off codex --off hooks` (or `--on …`).");
    Ok(())
}

fn parse_all(names: &[String]) -> Result<Vec<Choice>> {
    names
        .iter()
        .map(|name| choose::parse(name).ok_or_else(|| unknown(name, false)))
        .collect()
}

fn unknown(name: &str, assistants_only: bool) -> anyhow::Error {
    let assistants: Vec<&str> = sp_harness::known().into_iter().map(|(id, _)| id).collect();
    let mut known = assistants.join(", ");
    if !assistants_only {
        known.push_str(", instructions, skills, agents, commands, mcp, hooks, plugins");
    }
    Exit::with_code(
        2,
        format!("Unknown name `{name}`."),
        Some(&format!("Use one of: {known}.")),
    )
}

fn handed_back(released: &Released) {
    if released.files + released.entries > 0 {
        ui::hint(format!(
            "Handed back to the assistants: {} file(s) are real files again, {} config entr{} stay as they are.",
            released.files,
            released.entries,
            if released.entries == 1 { "y" } else { "ies" }
        ));
    }
}

fn next_sync(ctx: &Context) {
    if crate::daemon::running(ctx) {
        ui::hint("The background sync links them in a moment.");
    } else {
        ui::hint("They're linked on the next sync: `skillpouch sync`.");
    }
}
