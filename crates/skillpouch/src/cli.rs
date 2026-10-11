use clap::builder::styling::{Ansi256Color, AnsiColor, Color, Style, Styles};
use clap::{Parser, Subcommand};

/// Help text in the CLI's colours (clap turns them off for pipes and `NO_COLOR`).
const STYLES: Styles = {
    let accent = Some(Color::Ansi256(Ansi256Color(141)));
    Styles::styled()
        .header(Style::new().bold().fg_color(accent))
        .usage(Style::new().bold().fg_color(accent))
        .literal(Style::new().fg_color(accent))
        .placeholder(Style::new().dimmed())
        .valid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green))))
        .invalid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow))))
        .error(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::Red))),
        )
};

/// Sync your AI assistant skills, agents, MCP servers and instructions
/// across every computer, end-to-end encrypted.
#[derive(Debug, Parser)]
#[command(name = "skillpouch", version, propagate_version = true, styles = STYLES)]
pub struct Cli {
    /// Without a command: the banner and what to run next.
    #[command(subcommand)]
    pub command: Option<Command>,
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
    /// Sync this computer's pouch now.
    Sync,
    /// List or control the preferences synced for each assistant.
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Open the web app to pick versions of items changed in two places.
    Conflicts,
    /// Choose what this computer syncs: assistants and kinds, on or off.
    /// Without options, lists the current choice.
    Select {
        /// Turn on (an assistant id like `codex`, or `skills`, `mcp`, `hooks`…).
        #[arg(long, value_name = "NAME")]
        on: Vec<String>,
        /// Turn off; its items become the assistant's own files again.
        #[arg(long, value_name = "NAME")]
        off: Vec<String>,
    },
    /// Stop syncing one assistant, or this whole computer. Every file stays,
    /// as a real file instead of a link.
    Unlink {
        /// Only this assistant (`claude-code`, `codex`, …).
        #[arg(long, value_name = "ID")]
        assistant: Option<String>,
    },
    /// The background sync that keeps this computer up to date.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum SettingsAction {
    /// List entries and their status, optionally for one assistant.
    List {
        /// Only this assistant (`claude-code`, `opencode`, …).
        #[arg(value_name = "ASSISTANT")]
        harness: Option<String>,
    },
    /// Stop syncing an entry on this computer (the pouch keeps it).
    Exclude {
        /// The assistant (`claude-code`, `opencode`, …).
        #[arg(value_name = "ASSISTANT")]
        harness: String,
        /// The entry, as `settings list` shows it (`settings.json:env.API_URL`).
        entry: String,
    },
    /// Restore an excluded entry from the pouch now.
    Include {
        /// The assistant (`claude-code`, `opencode`, …).
        #[arg(value_name = "ASSISTANT")]
        harness: String,
        /// The entry, as `settings list` shows it (`settings.json:env.API_URL`).
        entry: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum DaemonAction {
    /// Start it, and start it again whenever you log in.
    Start,
    /// Stop it.
    Stop,
    /// Is it running, and how did the last sync go?
    Status,
    /// Run it in this terminal (what the service runs).
    #[command(hide = true)]
    Run {
        /// The SkillPouch folder (services can't always pass the environment).
        #[arg(long)]
        home: Option<std::path::PathBuf>,
        /// The server.
        #[arg(long)]
        api_url: Option<String>,
        /// Windows: start again without a console window, then exit.
        #[arg(long)]
        background: bool,
    },
}
