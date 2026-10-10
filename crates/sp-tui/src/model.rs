//! What the TUI shows and what it asks for. The CLI talks to the server and
//! sends [`Update`]s; the TUI answers keys with [`Request`]s.

use crate::usage::Usage;

/// Known before anything is fetched.
#[derive(Debug, Clone)]
pub struct Launch {
    pub version: String,
    pub server: String,
    /// This computer's hostname, the default name when it logs in.
    pub hostname: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub email: String,
    pub plan: String,
    pub pouch_limit: u64,
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub delete_after: Option<String>,
}

/// A pouch. Its name is end-to-end encrypted: without it the pouch shows by
/// number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pouch {
    pub id: String,
    pub number: usize,
    pub name: Option<String>,
    pub description: Option<String>,
    pub state: String,
    pub items: usize,
    /// More items than were counted.
    pub more: bool,
    /// "3 min ago".
    pub updated: Option<String>,
    pub event_seq: u64,
}

/// A computer connected to the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Computer {
    pub name: String,
    /// "Linux · x86_64".
    pub platform: String,
    pub cli_version: String,
    pub assistants: Vec<String>,
    pub key_storage: Option<String>,
    /// "3 min ago".
    pub last_seen: Option<String>,
    pub connected: Option<String>,
    pub state: String,
    pub current: bool,
    pub active_pouch: Option<String>,
    /// `(pouch id, last event it has)`.
    pub cursors: Vec<(String, u64)>,
}

impl Pouch {
    pub fn label(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("Pouch {}", self.number))
    }

    /// Only an active pouch can be synced; locked and archived ones can't.
    pub fn usable(&self) -> bool {
        self.state == "active"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dashboard {
    pub account: Account,
    pub pouches: Vec<Pouch>,
    pub computers: Vec<Computer>,
    pub browsers: usize,
    /// Why the pouch names couldn't be read, when they couldn't.
    pub names_missing: Option<String>,
    /// Token usage of every computer; `Err` says why it isn't there.
    /// `None` while this computer's logs are still being counted.
    pub usage: Option<Result<Usage, String>>,
    /// The first sync chosen at setup, while it hasn't run: `Merge with OpenCode`.
    pub first_sync: Option<String>,
    /// How syncing this computer is going; `None` before it is set up.
    pub sync: Option<SyncView>,
    /// What this computer syncs.
    pub choice: ChoiceView,
}

/// What this computer syncs, for the Sync tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChoiceView {
    /// The assistants found here.
    pub assistants: Vec<ChoiceRow>,
    /// Skills, agents, MCP servers…
    pub kinds: Vec<ChoiceRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRow {
    pub id: String,
    pub name: String,
    pub on: bool,
    /// `only read: part of a git checkout`.
    pub note: Option<String>,
}

/// One line about syncing, and maybe a second that explains it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncView {
    pub tone: SyncTone,
    /// `Synced 2 min ago`, `Syncing…`, `Offline`.
    pub text: String,
    pub detail: Option<String>,
    /// Web app path where open conflicts are resolved (`c` opens it).
    pub conflicts: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTone {
    Good,
    Busy,
    Warn,
    Bad,
}

impl Dashboard {
    pub fn this_computer(&self) -> Option<&Computer> {
        self.computers.iter().find(|computer| computer.current)
    }

    /// The one pouch this computer syncs.
    pub fn active_pouch(&self) -> Option<&Pouch> {
        let id = self.this_computer()?.active_pouch.as_ref()?;
        self.pouches.iter().find(|pouch| &pouch.id == id)
    }

    pub fn pouch(&self, id: &str) -> Option<&Pouch> {
        self.pouches.iter().find(|pouch| pouch.id == id)
    }

    pub fn live_pouches(&self) -> usize {
        self.pouches
            .iter()
            .filter(|pouch| pouch.state != "deleted")
            .count()
    }
}

/// The TUI asks the CLI to do something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Refresh,
    Login,
    CancelLogin,
    Logout,
    /// Turn an assistant (or, with `assistant: false`, a kind) on or off here.
    Choose {
        assistant: bool,
        id: String,
        on: bool,
    },
    /// Make this pouch the one this computer syncs.
    UsePouch(String),
    /// Open a web app path (`/app/account`) in the browser.
    Open(String),
    /// Look at this computer's assistants and set it up.
    Setup,
    FinishSetup(SetupChoice),
}

/// The pouch a computer syncs and how its first sync starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupChoice {
    pub pouch_id: String,
    pub first_sync: FirstSync,
    /// Assistants this computer leaves alone (ids).
    pub skip_assistants: Vec<String>,
    /// Kinds this computer doesn't sync (`hook`).
    pub skip_kinds: Vec<String>,
}

/// How the first sync treats what's in the pouch and on this computer.
/// Assistants are given by id (`claude-code`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstSync {
    /// The pouch is empty: fill it from this assistant, or start empty.
    Fill(Option<String>),
    /// This computer takes the pouch as it is.
    UsePouch,
    /// Both: the pouch's items and this assistant's.
    Merge(String),
    /// The pouch becomes what this assistant has.
    UseComputer(String),
}

/// The CLI tells the TUI what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// Token usage, counted after the rest of the dashboard.
    Usage(Result<Usage, String>),
    /// No keys for this server on this computer.
    SignedOut,
    Loaded(Box<Dashboard>),
    Failed(String),
    Login(LoginStep),
    Notice(Notice),
    /// This computer isn't set up: what its assistants hold.
    Setup(Vec<sp_harness::Harness>),
    /// Setup was saved (`Ok`), or why it wasn't.
    SetupSaved(Result<(), String>),
    /// The sync status changed.
    Sync(Option<SyncView>),
    /// How updating SkillPouch itself goes.
    Upgrade(Upgrade),
}

/// SkillPouch updating itself while the dashboard is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upgrade {
    Checking,
    /// Installing this version.
    Installing(String),
    /// This version is installed; it runs after a restart.
    Installed(String),
    /// Nothing newer, or nothing worth trying now.
    UpToDate,
    /// Why it couldn't be installed, and what to do about it.
    Failed {
        reason: String,
        hint: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStep {
    Starting,
    Waiting {
        url: String,
        code: String,
        computer: String,
        opened: bool,
    },
    Approved,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub tone: Tone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Success,
    Info,
    Error,
}

impl Notice {
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Success,
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Info,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Error,
        }
    }
}
