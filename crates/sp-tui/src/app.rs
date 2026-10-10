//! The TUI's state and what each key does. Drawing is in `view`.
//!
//! Three screens: the login screen while this computer isn't connected (or
//! is logging in again), setup right after it connects, and the dashboard.

use crate::model::{Dashboard, Launch, LoginStep, Notice, Request, Update};
use crate::setup::{self, Setup};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::{Duration, Instant};

/// How long a notice stays in the footer.
const NOTICE_FOR: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Pouches,
    Usage,
    Sync,
    Computers,
    Account,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Self::Overview,
        Self::Pouches,
        Self::Usage,
        Self::Sync,
        Self::Computers,
        Self::Account,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Pouches => "Pouches",
            Self::Usage => "Usage",
            Self::Sync => "Sync",
            Self::Computers => "Computers",
            Self::Account => "Account",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Loading,
    SignedOut,
    Ready(Box<Dashboard>),
    Failed(String),
}

/// A login in progress (or the screen offering one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    /// `None`: nothing started yet.
    pub step: Option<LoginStep>,
    pub since: Instant,
}

/// The actions on the Account tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAction {
    SetUp,
    OpenAccount,
    OpenBilling,
    Logout,
}

impl AccountAction {
    pub const ALL: [AccountAction; 4] = [
        Self::SetUp,
        Self::OpenAccount,
        Self::OpenBilling,
        Self::Logout,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SetUp => "Set up this computer again",
            Self::OpenAccount => "Open your account",
            Self::OpenBilling => "Manage plan and billing",
            Self::Logout => "Log out",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::SetUp => "Pick the pouch it syncs and how the first sync starts",
            Self::OpenAccount => "Profile, sign-in and security, in the browser",
            Self::OpenBilling => "Change or cancel your plan, in the browser",
            Self::Logout => "Deletes this computer's keys",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Send(Request),
    Quit,
}

pub struct App {
    pub launch: Launch,
    pub status: Status,
    /// Asked the server again; the current dashboard stays until it answers.
    pub refreshing: bool,
    pub tab: Tab,
    cursors: [usize; 6],
    /// The login screen is up.
    pub login: Option<Login>,
    /// Setting up what this computer syncs.
    pub setup: Option<Setup>,
    /// Asking before logging out.
    pub confirm_logout: bool,
    notice: Option<(Notice, Instant)>,
    pub tick: usize,
}

impl App {
    pub fn new(launch: Launch, signed_in: bool) -> Self {
        Self {
            launch,
            status: if signed_in {
                Status::Loading
            } else {
                Status::SignedOut
            },
            refreshing: false,
            tab: Tab::Overview,
            cursors: [0; 6],
            login: (!signed_in).then(Login::idle),
            setup: None,
            confirm_logout: false,
            notice: None,
            tick: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed() > NOTICE_FOR)
        {
            self.notice = None;
        }
    }

    pub fn dashboard(&self) -> Option<&Dashboard> {
        match &self.status {
            Status::Ready(dashboard) => Some(dashboard),
            _ => None,
        }
    }

    pub fn signed_in(&self) -> bool {
        !matches!(self.status, Status::SignedOut)
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref().map(|(notice, _)| notice)
    }

    pub fn cursor(&self, tab: Tab) -> usize {
        self.cursors[tab.index()]
    }

    fn rows(&self, tab: Tab) -> usize {
        match tab {
            Tab::Overview | Tab::Usage => 0,
            Tab::Pouches => self.dashboard().map_or(0, |d| d.pouches.len()),
            Tab::Computers => self.dashboard().map_or(0, |d| d.computers.len()),
            Tab::Sync => self
                .dashboard()
                .map_or(0, |d| d.choice.assistants.len() + d.choice.kinds.len()),
            Tab::Account => AccountAction::ALL.len(),
        }
    }

    fn clamp_cursors(&mut self) {
        for tab in Tab::ALL {
            let rows = self.rows(tab);
            let cursor = &mut self.cursors[tab.index()];
            *cursor = (*cursor).min(rows.saturating_sub(1));
        }
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::SignedOut => {
                self.status = Status::SignedOut;
                self.refreshing = false;
                self.login.get_or_insert_with(Login::idle);
            }
            Update::Loaded(mut dashboard) => {
                // Keep the last counts while new ones are worked out.
                if dashboard.usage.is_none()
                    && let Some(old) = self.dashboard()
                {
                    dashboard.usage = old.usage.clone();
                }
                self.status = Status::Ready(dashboard);
                self.refreshing = false;
            }
            Update::Usage(usage) => {
                if let Status::Ready(dashboard) = &mut self.status {
                    dashboard.usage = Some(usage);
                }
            }
            Update::Failed(message) => {
                self.refreshing = false;
                if self.dashboard().is_some() {
                    self.show(Notice::error(message));
                } else {
                    self.status = Status::Failed(message);
                }
            }
            Update::Sync(sync) => {
                if let Status::Ready(dashboard) = &mut self.status {
                    dashboard.sync = sync;
                }
            }
            Update::Login(step) => self.login_step(step),
            Update::Notice(notice) => self.show(notice),
            Update::Setup(assistants) => {
                if self.setup.is_none()
                    && let Some(dashboard) = self.dashboard()
                {
                    self.setup = Some(Setup::new(assistants, dashboard));
                }
            }
            Update::SetupSaved(Ok(())) => {
                let label =
                    self.setup
                        .take()
                        .zip(self.dashboard())
                        .and_then(|(setup, dashboard)| {
                            setup.chosen_pouch(dashboard).map(|p| p.label())
                        });
                self.tab = Tab::Overview;
                if let Some(label) = label {
                    self.show(Notice::success(format!(
                        "Set up. This computer syncs {label}."
                    )));
                }
            }
            Update::SetupSaved(Err(message)) => {
                if let Some(setup) = &mut self.setup {
                    setup.saving = false;
                    setup.error = Some(message);
                }
            }
        }
        self.clamp_cursors();
    }

    fn login_step(&mut self, step: LoginStep) {
        match step {
            LoginStep::Approved => {
                self.login = None;
                self.status = Status::Loading;
                self.tab = Tab::Overview;
                self.show(Notice::success("Logged in. This computer is connected."));
            }
            LoginStep::Cancelled => self.show(Notice::info("Login cancelled. Nothing was saved.")),
            step => {
                // A login the user already left doesn't come back.
                if let Some(Login { step: current, .. }) = &mut self.login
                    && current.is_some()
                {
                    *current = Some(step);
                }
            }
        }
    }

    fn show(&mut self, notice: Notice) {
        self.notice = Some((notice, Instant::now()));
    }

    fn start_login(&mut self) -> Outcome {
        self.login = Some(Login {
            step: Some(LoginStep::Starting),
            since: Instant::now(),
        });
        Outcome::Send(Request::Login)
    }

    fn open(&self, path: &str) -> Outcome {
        Outcome::Send(Request::Open(format!("{}{path}", self.launch.server)))
    }

    /// Where `o` goes from the current tab.
    fn web_path(&self) -> String {
        match self.tab {
            Tab::Overview => "/app".into(),
            Tab::Usage => "/app/account/usage".into(),
            Tab::Pouches => self
                .dashboard()
                .and_then(|d| d.pouches.get(self.cursor(Tab::Pouches)))
                .map_or("/app".into(), |pouch| format!("/app/p/{}", pouch.id)),
            Tab::Sync => "/app".into(),
            Tab::Computers => "/app/account/devices".into(),
            Tab::Account => "/app/account".into(),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        if key.kind != KeyEventKind::Press {
            return Outcome::Nothing;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Outcome::Quit;
        }
        if let Some(login) = self.login.clone() {
            return self.on_login_key(key.code, login.step.as_ref());
        }
        if self.setup.is_some() {
            return self.on_setup_key(key.code);
        }
        if self.confirm_logout {
            return self.on_logout_key(key.code);
        }
        self.on_dashboard_key(key.code)
    }

    fn on_login_key(&mut self, code: KeyCode, step: Option<&LoginStep>) -> Outcome {
        match (code, step) {
            (KeyCode::Char('q'), None | Some(LoginStep::Failed(_))) => Outcome::Quit,
            (KeyCode::Enter, None | Some(LoginStep::Failed(_))) => self.start_login(),
            (KeyCode::Char('o'), Some(LoginStep::Waiting { url, .. })) => {
                Outcome::Send(Request::Open(url.clone()))
            }
            // Back: to the dashboard when there is one, else to the start.
            (KeyCode::Esc, Some(step)) => {
                let waiting = !matches!(step, LoginStep::Failed(_));
                self.login = (!self.signed_in()).then(Login::idle);
                if waiting {
                    Outcome::Send(Request::CancelLogin)
                } else {
                    Outcome::Nothing
                }
            }
            (KeyCode::Esc, None) if self.signed_in() => {
                self.login = None;
                Outcome::Nothing
            }
            (KeyCode::Esc, None) => Outcome::Quit,
            _ => Outcome::Nothing,
        }
    }

    fn on_setup_key(&mut self, code: KeyCode) -> Outcome {
        let (Some(setup), Status::Ready(dashboard)) = (&mut self.setup, &self.status) else {
            self.setup = None;
            return Outcome::Nothing;
        };
        if code == KeyCode::Char('q') && !setup.saving {
            return Outcome::Quit;
        }
        match setup.on_key(code, dashboard) {
            setup::Outcome::Stay => Outcome::Nothing,
            setup::Outcome::Send(request) => Outcome::Send(request),
            setup::Outcome::Close => {
                self.setup = None;
                self.show(Notice::info(
                    "Skipped. Set this computer up any time from the Account tab.",
                ));
                Outcome::Nothing
            }
        }
    }

    fn on_logout_key(&mut self, code: KeyCode) -> Outcome {
        self.confirm_logout = false;
        match code {
            KeyCode::Char('y') | KeyCode::Enter => Outcome::Send(Request::Logout),
            _ => Outcome::Nothing,
        }
    }

    fn on_dashboard_key(&mut self, code: KeyCode) -> Outcome {
        let index = self.tab.index();
        let count = Tab::ALL.len();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => Outcome::Quit,
            KeyCode::Char(digit @ '1'..='6') => {
                self.tab = Tab::ALL[usize::from(digit as u8 - b'1')];
                Outcome::Nothing
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.tab = Tab::ALL[(index + 1) % count];
                Outcome::Nothing
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.tab = Tab::ALL[(index + count - 1) % count];
                Outcome::Nothing
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let rows = self.rows(self.tab);
                if rows > 0 {
                    self.cursors[index] = (self.cursors[index] + 1) % rows;
                }
                Outcome::Nothing
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let rows = self.rows(self.tab);
                if rows > 0 {
                    self.cursors[index] = self.cursors[index].checked_sub(1).unwrap_or(rows - 1);
                }
                Outcome::Nothing
            }
            KeyCode::Char('r') => {
                self.refreshing = true;
                Outcome::Send(Request::Refresh)
            }
            KeyCode::Char('o') => self.open(&self.web_path()),
            KeyCode::Char('c') => match self.conflicts_path() {
                Some(path) => self.open(&path),
                None => Outcome::Nothing,
            },
            KeyCode::Enter | KeyCode::Char(' ') if self.tab == Tab::Sync => self.toggle_choice(),
            KeyCode::Enter if self.tab == Tab::Pouches => self.use_selected_pouch(),
            KeyCode::Enter if self.tab == Tab::Account => {
                match AccountAction::ALL.get(self.cursor(Tab::Account)) {
                    Some(AccountAction::SetUp) => Outcome::Send(Request::Setup),
                    Some(AccountAction::OpenAccount) => self.open("/app/account"),
                    Some(AccountAction::OpenBilling) => self.open("/app/account/billing"),
                    Some(AccountAction::Logout) => {
                        self.confirm_logout = true;
                        Outcome::Nothing
                    }
                    None => Outcome::Nothing,
                }
            }
            _ => Outcome::Nothing,
        }
    }
}

impl App {
    /// Makes the selected pouch the one this computer syncs. The dashboard
    /// shows it right away; the next load confirms it.
    /// Where the open conflicts are resolved, when there are any.
    pub fn conflicts_path(&self) -> Option<String> {
        self.dashboard()?.sync.as_ref()?.conflicts.clone()
    }

    /// Flips the selected assistant or kind on the Sync tab (shown at once;
    /// the CLI saves it and hands items back to the assistant).
    fn toggle_choice(&mut self) -> Outcome {
        let cursor = self.cursor(Tab::Sync);
        let Status::Ready(dashboard) = &mut self.status else {
            return Outcome::Nothing;
        };
        let choice = &mut dashboard.choice;
        let assistant = cursor < choice.assistants.len();
        let last_kind = choice.kinds.iter().filter(|row| row.on).count() == 1;
        let row = if assistant {
            choice.assistants.get_mut(cursor)
        } else {
            choice.kinds.get_mut(cursor - choice.assistants.len())
        };
        let Some(row) = row else {
            return Outcome::Nothing;
        };
        if !assistant && row.on && last_kind {
            self.show(Notice::error(
                "Leave at least one kind on, or unlink this computer from the Account tab.",
            ));
            return Outcome::Nothing;
        }
        row.on = !row.on;
        let request = Request::Choose {
            assistant,
            id: row.id.clone(),
            on: row.on,
        };
        let text = if row.on {
            format!("{} on: linked on the next sync.", row.name)
        } else {
            format!("{} off: its files are its own again.", row.name)
        };
        self.show(Notice::info(text));
        Outcome::Send(request)
    }

    fn use_selected_pouch(&mut self) -> Outcome {
        let cursor = self.cursor(Tab::Pouches);
        let Status::Ready(dashboard) = &mut self.status else {
            return Outcome::Nothing;
        };
        let Some(pouch) = dashboard.pouches.get(cursor) else {
            return Outcome::Nothing;
        };
        let (id, label) = (pouch.id.clone(), pouch.label());
        if !pouch.usable() {
            let state = pouch.state.clone();
            self.show(Notice::error(format!(
                "{label} is {state}, so it can't sync. Manage it in the web app."
            )));
            return Outcome::Nothing;
        }
        let Some(this) = dashboard.computers.iter_mut().find(|c| c.current) else {
            return Outcome::Nothing;
        };
        if this.active_pouch.as_deref() == Some(id.as_str()) {
            return Outcome::Nothing;
        }
        this.active_pouch = Some(id.clone());
        self.show(Notice::info(format!("This computer now syncs {label}.")));
        Outcome::Send(Request::UsePouch(id))
    }
}

impl Login {
    fn idle() -> Self {
        Self {
            step: None,
            since: Instant::now(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Account, Computer, Pouch};

    pub fn launch() -> Launch {
        Launch {
            version: "v0.1.0".into(),
            server: "https://skillpouch.net".into(),
            hostname: "laptop".into(),
        }
    }

    pub fn dashboard() -> Dashboard {
        Dashboard {
            account: Account {
                name: "Ada Lovelace".into(),
                email: "ada@example.com".into(),
                plan: "Supporter".into(),
                pouch_limit: 3,
                used_bytes: 1536,
                quota_bytes: 1_073_741_824,
                delete_after: None,
            },
            pouches: vec![
                Pouch {
                    id: "0193ab12-0000-7000-8000-000000000001".into(),
                    number: 1,
                    name: Some("Work stuff".into()),
                    description: Some("Skills for the day job".into()),
                    state: "active".into(),
                    items: 12,
                    more: false,
                    updated: Some("3 min ago".into()),
                    event_seq: 40,
                },
                Pouch {
                    id: "0193ab12-0000-7000-8000-000000000002".into(),
                    number: 2,
                    name: None,
                    description: None,
                    state: "locked".into(),
                    items: 2,
                    more: false,
                    updated: None,
                    event_seq: 3,
                },
            ],
            computers: vec![
                Computer {
                    name: "laptop".into(),
                    platform: "Linux x86_64".into(),
                    cli_version: "0.1.0".into(),
                    assistants: vec!["Claude Code".into(), "OpenCode".into()],
                    key_storage: Some("file".into()),
                    last_seen: Some("just now".into()),
                    connected: Some("Oct 8".into()),
                    state: "active".into(),
                    current: true,
                    active_pouch: Some("0193ab12-0000-7000-8000-000000000001".into()),
                    cursors: vec![("0193ab12-0000-7000-8000-000000000001".into(), 40)],
                },
                Computer {
                    name: "desktop".into(),
                    platform: "Windows x86_64".into(),
                    cli_version: "0.1.0".into(),
                    assistants: vec![],
                    key_storage: None,
                    last_seen: Some("2 days ago".into()),
                    connected: None,
                    state: "active".into(),
                    current: false,
                    active_pouch: None,
                    cursors: vec![("0193ab12-0000-7000-8000-000000000001".into(), 31)],
                },
            ],
            browsers: 1,
            names_missing: None,
            usage: Some(Ok(crate::usage::tests::usage())),
            first_sync: None,
            sync: None,
            choice: crate::model::ChoiceView {
                assistants: vec![
                    crate::model::ChoiceRow {
                        id: "claude-code".into(),
                        name: "Claude Code".into(),
                        on: true,
                        note: None,
                    },
                    crate::model::ChoiceRow {
                        id: "opencode".into(),
                        name: "OpenCode".into(),
                        on: true,
                        note: Some("only read: part of a git checkout".into()),
                    },
                ],
                kinds: vec![
                    crate::model::ChoiceRow {
                        id: "skill".into(),
                        name: "Skills".into(),
                        on: true,
                        note: None,
                    },
                    crate::model::ChoiceRow {
                        id: "hook".into(),
                        name: "Hooks".into(),
                        on: true,
                        note: None,
                    },
                ],
            },
        }
    }

    pub fn ready() -> App {
        let mut app = App::new(launch(), true);
        app.apply(Update::Loaded(Box::new(dashboard())));
        app
    }

    fn press(app: &mut App, code: KeyCode) -> Outcome {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn usage_arrives_after_the_dashboard_and_survives_a_refresh() {
        let mut app = App::new(launch(), true);
        let mut first = dashboard();
        first.usage = None;
        app.apply(Update::Loaded(Box::new(first.clone())));
        assert!(app.dashboard().is_some_and(|d| d.usage.is_none()));
        app.apply(Update::Usage(Ok(crate::usage::tests::usage())));
        assert!(
            app.dashboard()
                .is_some_and(|d| matches!(d.usage, Some(Ok(_))))
        );
        // A refresh counts again; the last counts stay meanwhile.
        app.apply(Update::Loaded(Box::new(first)));
        assert!(
            app.dashboard()
                .is_some_and(|d| matches!(d.usage, Some(Ok(_))))
        );
    }

    #[test]
    fn numbers_tab_and_arrows_switch_tabs() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.tab, Tab::Usage);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Sync);
        for _ in 0..4 {
            press(&mut app, KeyCode::Left);
        }
        assert_eq!(app.tab, Tab::Account);
    }

    #[test]
    fn lists_wrap_and_open_the_selected_pouch() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Up);
        assert_eq!(app.cursor(Tab::Pouches), 1);
        assert_eq!(
            press(&mut app, KeyCode::Char('o')),
            Outcome::Send(Request::Open(
                "https://skillpouch.net/app/p/0193ab12-0000-7000-8000-000000000002".into()
            ))
        );
    }

    #[test]
    fn enter_picks_the_pouch_this_computer_syncs() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('2'));
        // Already the active one: nothing to do.
        assert_eq!(press(&mut app, KeyCode::Enter), Outcome::Nothing);

        let mut second = dashboard();
        second.pouches[1].state = "active".into();
        app.apply(Update::Loaded(Box::new(second)));
        press(&mut app, KeyCode::Down);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Outcome::Send(Request::UsePouch(
                "0193ab12-0000-7000-8000-000000000002".into()
            ))
        );
        let active = app.dashboard().and_then(Dashboard::active_pouch);
        assert_eq!(active.map(|p| p.number), Some(2));
        assert_eq!(
            app.notice().map(|n| n.text.as_str()),
            Some("This computer now syncs Pouch 2.")
        );
    }

    #[test]
    fn a_locked_pouch_cant_be_picked() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Enter), Outcome::Nothing);
        assert_eq!(
            app.dashboard()
                .and_then(Dashboard::active_pouch)
                .map(|p| p.number),
            Some(1)
        );
        assert!(app.notice().is_some_and(|n| n.text.contains("locked")));
    }

    #[test]
    fn a_new_computer_only_gets_the_login_screen() {
        let mut app = App::new(launch(), false);
        assert!(app.login.is_some());
        // Dashboard keys do nothing here.
        assert_eq!(press(&mut app, KeyCode::Char('2')), Outcome::Nothing);
        assert_eq!(app.tab, Tab::Overview);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Outcome::Send(Request::Login)
        );
        app.apply(Update::Login(LoginStep::Waiting {
            url: "https://skillpouch.net/cli/authorize?r=1".into(),
            code: "2605-3825".into(),
            computer: "laptop".into(),
            opened: true,
        }));
        assert_eq!(
            press(&mut app, KeyCode::Char('o')),
            Outcome::Send(Request::Open(
                "https://skillpouch.net/cli/authorize?r=1".into()
            ))
        );
        app.apply(Update::Login(LoginStep::Approved));
        assert_eq!(app.login, None);
        assert_eq!(app.status, Status::Loading);
    }

    #[test]
    fn escape_cancels_a_login_and_a_failed_one_can_be_retried() {
        let mut app = App::new(launch(), false);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            press(&mut app, KeyCode::Esc),
            Outcome::Send(Request::CancelLogin)
        );
        assert_eq!(app.login.as_ref().and_then(|l| l.step.clone()), None);
        // A late answer for the cancelled login doesn't come back.
        app.apply(Update::Login(LoginStep::Failed("Expired.".into())));
        assert_eq!(app.login.as_ref().and_then(|l| l.step.clone()), None);

        press(&mut app, KeyCode::Enter);
        app.apply(Update::Login(LoginStep::Failed("Expired.".into())));
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Outcome::Send(Request::Login)
        );
    }

    #[test]
    fn the_account_tab_offers_setup_the_web_and_log_out() {
        let labels: Vec<&str> = AccountAction::ALL.iter().map(|a| a.label()).collect();
        assert_eq!(
            labels,
            [
                "Set up this computer again",
                "Open your account",
                "Manage plan and billing",
                "Log out"
            ]
        );
        let mut app = ready();
        press(&mut app, KeyCode::Char('6'));
        for _ in 0..3 {
            press(&mut app, KeyCode::Down);
        }
        press(&mut app, KeyCode::Enter);
        assert!(app.confirm_logout && app.login.is_none());
    }

    #[test]
    fn the_sync_tab_turns_assistants_and_kinds_on_and_off() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Outcome::Send(Request::Choose {
                assistant: true,
                id: "claude-code".into(),
                on: false,
            })
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(
            press(&mut app, KeyCode::Char(' ')),
            Outcome::Send(Request::Choose {
                assistant: false,
                id: "skill".into(),
                on: false,
            })
        );
        // The last kind on stays on.
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Enter), Outcome::Nothing);
        assert!(
            app.notice()
                .is_some_and(|n| n.text.contains("at least one"))
        );
    }

    #[test]
    fn logout_asks_first() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('6'));
        press(&mut app, KeyCode::Up);
        assert_eq!(press(&mut app, KeyCode::Enter), Outcome::Nothing);
        assert!(app.confirm_logout);
        assert_eq!(press(&mut app, KeyCode::Char('n')), Outcome::Nothing);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            press(&mut app, KeyCode::Char('y')),
            Outcome::Send(Request::Logout)
        );
        app.apply(Update::SignedOut);
        assert!(app.login.is_some());
    }

    #[test]
    fn setup_opens_once_takes_the_keys_and_closes_when_saved() {
        let mut app = ready();
        app.apply(Update::Setup(crate::setup::tests::assistants()));
        assert!(app.setup.is_some());
        // A second offer (a refresh) doesn't restart it.
        press(&mut app, KeyCode::Enter);
        app.apply(Update::Setup(Vec::new()));
        assert_eq!(app.setup.as_ref().map(|s| s.assistants.len()), Some(2));
        // Dashboard keys don't reach the tabs.
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.tab, Tab::Overview);
        // Use the pouch, keep everything on, save.
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            press(&mut app, KeyCode::Enter),
            Outcome::Send(Request::FinishSetup(_))
        ));
        app.apply(Update::SetupSaved(Err("Server unreachable.".into())));
        let failed = app.setup.as_ref().map(|s| (s.saving, s.error.clone()));
        assert_eq!(failed, Some((false, Some("Server unreachable.".into()))));
        press(&mut app, KeyCode::Enter);
        app.apply(Update::SetupSaved(Ok(())));
        assert!(app.setup.is_none());
        assert_eq!(
            app.notice().map(|n| n.text.as_str()),
            Some("Set up. This computer syncs Work stuff.")
        );
    }

    #[test]
    fn setup_can_be_skipped_and_reopened_from_the_account_tab() {
        let mut app = ready();
        app.apply(Update::Setup(Vec::new()));
        press(&mut app, KeyCode::Esc);
        assert!(app.setup.is_none());
        assert!(app.notice().is_some_and(|n| n.text.starts_with("Skipped")));
        press(&mut app, KeyCode::Char('6'));
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Outcome::Send(Request::Setup)
        );
    }

    #[test]
    fn a_failed_refresh_keeps_the_dashboard() {
        let mut app = ready();
        assert_eq!(
            press(&mut app, KeyCode::Char('r')),
            Outcome::Send(Request::Refresh)
        );
        app.apply(Update::Failed("Server unreachable.".into()));
        assert!(app.dashboard().is_some());
        assert_eq!(
            app.notice().map(|n| n.text.as_str()),
            Some("Server unreachable.")
        );
    }

    #[test]
    fn cursors_stay_inside_shorter_lists() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Down);
        let mut smaller = dashboard();
        smaller.pouches.truncate(1);
        app.apply(Update::Loaded(Box::new(smaller)));
        assert_eq!(app.cursor(Tab::Pouches), 0);
    }
}
