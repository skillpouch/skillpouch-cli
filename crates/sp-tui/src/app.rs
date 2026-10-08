//! The TUI's state and what each key does. Drawing is in `view`.
//!
//! Two screens: the login screen while this computer isn't connected (or is
//! logging in again), and the dashboard once it is.

use crate::model::{Dashboard, Launch, LoginStep, Notice, Request, Update};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::{Duration, Instant};

/// How long a notice stays in the footer.
const NOTICE_FOR: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Pouches,
    Computers,
    Assistants,
    Account,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Self::Overview,
        Self::Pouches,
        Self::Computers,
        Self::Assistants,
        Self::Account,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Pouches => "Pouches",
            Self::Computers => "Computers",
            Self::Assistants => "Assistants",
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
    OpenAccount,
    OpenBilling,
    Relogin,
    Logout,
}

impl AccountAction {
    pub const ALL: [AccountAction; 4] = [
        Self::OpenAccount,
        Self::OpenBilling,
        Self::Relogin,
        Self::Logout,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAccount => "Open your account",
            Self::OpenBilling => "Manage plan and billing",
            Self::Relogin => "Log in again with new keys",
            Self::Logout => "Log out",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::OpenAccount => "Profile, sign-in and security, in the browser",
            Self::OpenBilling => "Change or cancel your plan, in the browser",
            Self::Relogin => "Replaces this computer's keys once you approve",
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
    cursors: [usize; 5],
    /// The login screen is up.
    pub login: Option<Login>,
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
            cursors: [0; 5],
            login: (!signed_in).then(Login::idle),
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
            Tab::Overview => 0,
            Tab::Pouches => self.dashboard().map_or(0, |d| d.pouches.len()),
            Tab::Computers => self.dashboard().map_or(0, |d| d.computers.len()),
            Tab::Assistants => self.launch.assistants.len(),
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
            Update::Loaded(dashboard) => {
                self.status = Status::Ready(dashboard);
                self.refreshing = false;
            }
            Update::Failed(message) => {
                self.refreshing = false;
                if self.dashboard().is_some() {
                    self.show(Notice::error(message));
                } else {
                    self.status = Status::Failed(message);
                }
            }
            Update::Login(step) => self.login_step(step),
            Update::Notice(notice) => self.show(notice),
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
            Tab::Overview | Tab::Assistants => "/app".into(),
            Tab::Pouches => self
                .dashboard()
                .and_then(|d| d.pouches.get(self.cursor(Tab::Pouches)))
                .map_or("/app".into(), |pouch| format!("/app/p/{}", pouch.id)),
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
            KeyCode::Char(digit @ '1'..='5') => {
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
            KeyCode::Enter if self.tab == Tab::Account => {
                match AccountAction::ALL.get(self.cursor(Tab::Account)) {
                    Some(AccountAction::OpenAccount) => self.open("/app/account"),
                    Some(AccountAction::OpenBilling) => self.open("/app/account/billing"),
                    Some(AccountAction::Relogin) => {
                        self.login = Some(Login::idle());
                        Outcome::Nothing
                    }
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
    use crate::model::{Account, Assistant, Computer, Pouch};

    pub fn launch() -> Launch {
        Launch {
            version: "v0.1.0".into(),
            server: "https://skillpouch.net".into(),
            hostname: "laptop".into(),
            assistants: vec![
                Assistant {
                    name: "Claude Code".into(),
                    folder: ".claude".into(),
                    installed: true,
                },
                Assistant {
                    name: "Codex".into(),
                    folder: ".codex".into(),
                    installed: false,
                },
            ],
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
                    state: "active".into(),
                    items: 12,
                    more: false,
                    updated: Some("3 min ago".into()),
                    event_seq: 40,
                },
                Pouch {
                    id: "0193ab12-0000-7000-8000-000000000002".into(),
                    number: 2,
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
    fn numbers_tab_and_arrows_switch_tabs() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.tab, Tab::Computers);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Assistants);
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
    fn logging_in_again_can_be_left_for_the_dashboard() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('5'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert!(app.login.is_some());
        press(&mut app, KeyCode::Esc);
        assert!(app.login.is_none());
    }

    #[test]
    fn logout_asks_first() {
        let mut app = ready();
        press(&mut app, KeyCode::Char('5'));
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
