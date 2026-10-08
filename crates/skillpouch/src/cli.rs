use clap::{Parser, Subcommand};

/// Sync your AI assistant skills, agents, MCP servers and instructions
/// across every computer, end-to-end encrypted.
#[derive(Debug, Parser)]
#[command(name = "skillpouch", version, propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Connect this computer to your SkillPouch account.
    Login {
        /// One-time token from "Connect a computer" in the web app.
        token: Option<String>,
        /// Name shown on your Devices page (default: this computer's name).
        #[arg(long)]
        name: Option<String>,
        /// Print the approval link instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Forget this computer's keys and tokens.
    Logout,
    /// Show the account and device this computer is connected to.
    Status {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}
