//! What the TUI shows and what it asks for. The CLI talks to the server and
//! sends [`Update`]s; the TUI answers keys with [`Request`]s.

/// Known before anything is fetched.
#[derive(Debug, Clone)]
pub struct Launch {
    pub version: String,
    pub server: String,
    /// This computer's hostname, the default name when it logs in.
    pub hostname: String,
    pub assistants: Vec<Assistant>,
}

/// An AI assistant on this computer, found by its config folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assistant {
    pub name: String,
    pub folder: String,
    pub installed: bool,
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

/// A pouch as the server sees it. Its name and items are end-to-end
/// encrypted, so it is shown by number until this computer can decrypt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pouch {
    pub id: String,
    pub number: usize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dashboard {
    pub account: Account,
    pub pouches: Vec<Pouch>,
    pub computers: Vec<Computer>,
    pub browsers: usize,
}

impl Dashboard {
    pub fn this_computer(&self) -> Option<&Computer> {
        self.computers.iter().find(|computer| computer.current)
    }

    pub fn items(&self) -> usize {
        self.pouches.iter().map(|pouch| pouch.items).sum()
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
    /// Open a web app path (`/app/account`) in the browser.
    Open(String),
}

/// The CLI tells the TUI what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// No keys for this server on this computer.
    SignedOut,
    Loaded(Box<Dashboard>),
    Failed(String),
    Login(LoginStep),
    Notice(Notice),
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
