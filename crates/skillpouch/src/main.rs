mod browser;
mod cli;
mod commands;
mod context;
mod daemon;
mod exit;
mod names;
mod setup;
mod sync;
mod ui;
mod usage;
mod when;

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return exit::report(&error.into()),
    };
    match runtime.block_on(commands::run(cli)) {
        Ok(code) => code,
        Err(error) => exit::report(&error),
    }
}
