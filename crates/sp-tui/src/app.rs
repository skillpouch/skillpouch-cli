//! The home screen: the logo, the account this computer is connected to, and
//! what to do next. Commands that print (log in, log out) run after the TUI
//! closes, so their output stays in the terminal.

use crate::logo::{self, Mark};
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{FilterType, Resize, StatefulImage};

/// Logo size in cells: about square on screen.
const LOGO_COLUMNS: u16 = 20;
const LOGO_ROWS: u16 = 11;
/// The screen never grows wider than this.
const MAX_WIDTH: u16 = 92;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What the TUI was opened with.
#[derive(Debug, Clone)]
pub struct Launch {
    pub version: String,
    pub server: String,
}

/// The signed-in account, as `skillpouch status` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub email: String,
    pub computer: Option<String>,
    pub plan: String,
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub delete_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// No keys for this server on this computer.
    NotConnected,
    Checking,
    Connected(Account),
    Failed(String),
}

/// What the user picked; the CLI carries it out once the TUI has closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Login,
    Logout,
    /// Ask the server again (handled inside the TUI).
    Refresh,
}

impl Action {
    fn label(self, connection: &Connection) -> &'static str {
        match (self, connection) {
            (Self::Login, Connection::NotConnected) => "Log in",
            (Self::Login, _) => "Log in again with new keys",
            (Self::Logout, _) => "Log out",
            (Self::Refresh, Connection::Failed(_)) => "Try again",
            (Self::Refresh, _) => "Refresh",
            (Self::Quit, _) => "Quit",
        }
    }

    fn detail(self, connection: &Connection) -> &'static str {
        match (self, connection) {
            (Self::Login, Connection::NotConnected) => "Connect this computer to your account",
            (Self::Login, _) => "Replace this computer's keys",
            (Self::Logout, _) => "Forget this computer's keys",
            (Self::Refresh, _) => "Check the account again",
            (Self::Quit, _) => "Back to the terminal",
        }
    }
}

/// How the logo is drawn.
pub enum LogoView {
    /// The real picture (sixel, kitty or iTerm2).
    Picture(Box<StatefulProtocol>),
    /// Traced in half-blocks.
    Blocks(Mark),
}

pub struct App {
    launch: Launch,
    connection: Connection,
    logo: LogoView,
    selected: usize,
    tick: usize,
}

impl App {
    pub fn new(launch: Launch, connection: Connection, logo: LogoView) -> Self {
        Self {
            launch,
            connection,
            logo,
            selected: 0,
            tick: 0,
        }
    }

    pub fn set_connection(&mut self, connection: Connection) {
        self.connection = connection;
        self.selected = self.selected.min(self.actions().len() - 1);
    }

    /// Advances the spinner.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn actions(&self) -> Vec<Action> {
        match self.connection {
            Connection::NotConnected => vec![Action::Login, Action::Quit],
            Connection::Checking => vec![Action::Quit],
            Connection::Connected(_) | Connection::Failed(_) => {
                vec![Action::Refresh, Action::Login, Action::Logout, Action::Quit]
            }
        }
    }

    /// The action a key asks for, if any.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let actions = self.actions();
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Quit)
            }
            KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.checked_sub(1).unwrap_or(actions.len() - 1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1) % actions.len();
                None
            }
            KeyCode::Enter | KeyCode::Char(' ') => actions.get(self.selected).copied(),
            _ => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = centered(frame.area(), MAX_WIDTH);
        let [header, _, body, footer] = Layout::vertical([
            Constraint::Length(LOGO_ROWS),
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_header(frame, header);

        let [account, actions] = Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)])
            .spacing(1)
            .areas(body);
        self.draw_account(frame, account);
        self.draw_actions(frame, actions);

        let keys = Line::from(vec![
            Span::styled("↑↓", theme::accent()),
            Span::styled(" move   ", theme::dim()),
            Span::styled("enter", theme::accent()),
            Span::styled(" select   ", theme::dim()),
            Span::styled("q", theme::accent()),
            Span::styled(" quit", theme::dim()),
        ]);
        frame.render_widget(Paragraph::new(keys).centered(), footer);
    }

    fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let [logo_area, _, text] = Layout::horizontal([
            Constraint::Length(LOGO_COLUMNS),
            Constraint::Length(3),
            Constraint::Fill(1),
        ])
        .areas(area);
        match &mut self.logo {
            LogoView::Picture(protocol) => {
                let image =
                    StatefulImage::default().resize(Resize::Fit(Some(FilterType::Lanczos3)));
                frame.render_stateful_widget(image, logo_area, protocol.as_mut());
            }
            LogoView::Blocks(mark) => frame.render_widget(MarkWidget(mark), logo_area),
        }
        let lines = vec![
            Line::default(),
            Line::default(),
            Line::default(),
            Line::default(),
            Line::from(vec![
                Span::styled("SkillPouch", theme::brand().bold()),
                Span::styled(format!("  {}", self.launch.version), theme::dim()),
            ]),
            Line::from("Your skills, agents and MCP servers on every computer."),
            Line::styled("End-to-end encrypted.", theme::dim()),
        ];
        frame.render_widget(Paragraph::new(lines), text);
    }

    fn draw_account(&self, frame: &mut Frame, area: Rect) {
        let block = panel("Account");
        let rows: Vec<Line> = match &self.connection {
            Connection::NotConnected => vec![
                Line::from("This computer isn't connected yet."),
                Line::default(),
                row("Server", Span::styled(&self.launch.server, theme::dim())),
            ],
            Connection::Checking => vec![
                Line::from(vec![
                    Span::styled(SPINNER[self.tick % SPINNER.len()], theme::accent()),
                    Span::raw("  Checking your account…"),
                ]),
                Line::default(),
                row("Server", Span::styled(&self.launch.server, theme::dim())),
            ],
            Connection::Failed(message) => vec![
                Line::styled(format!("✖  {message}"), theme::error()),
                Line::default(),
                row("Server", Span::styled(&self.launch.server, theme::dim())),
            ],
            Connection::Connected(account) => account_rows(account, &self.launch.server),
        };
        frame.render_widget(Paragraph::new(rows).block(block), area);
    }

    fn draw_actions(&self, frame: &mut Frame, area: Rect) {
        let actions = self.actions();
        let mut lines = Vec::new();
        for (index, action) in actions.iter().enumerate() {
            let selected = index == self.selected;
            let marker = if selected { "› " } else { "  " };
            let label = action.label(&self.connection);
            lines.push(if selected {
                Line::from(vec![
                    Span::styled(marker, theme::accent()),
                    Span::styled(label, theme::accent().bold()),
                ])
            } else {
                Line::from(vec![Span::raw(marker), Span::raw(label)])
            });
            if selected {
                lines.push(Line::styled(
                    format!("  {}", action.detail(&self.connection)),
                    theme::dim(),
                ));
            }
        }
        frame.render_widget(Paragraph::new(lines).block(panel("Actions")), area);
    }
}

fn account_rows<'a>(account: &'a Account, server: &'a str) -> Vec<Line<'a>> {
    let mut rows = vec![
        row(
            "Signed in",
            Span::styled(account.name.as_str(), Style::new().bold()),
        ),
        row("", Span::styled(account.email.as_str(), theme::dim())),
    ];
    if let Some(computer) = &account.computer {
        rows.push(row("Computer", Span::raw(computer.as_str())));
    }
    rows.push(Line::from(vec![
        label("Plan"),
        Span::styled("✦ ", theme::accent()),
        Span::raw(account.plan.as_str()),
    ]));
    let mut storage = vec![label("Storage")];
    storage.extend(meter(account.used_bytes, account.quota_bytes, 12));
    storage.push(Span::raw(format!("  {}", bytes(account.used_bytes))));
    storage.push(Span::styled(
        format!(" of {}", bytes(account.quota_bytes)),
        theme::dim(),
    ));
    rows.push(Line::from(storage));
    rows.push(row("Server", Span::styled(server, theme::dim())));
    if let Some(date) = &account.delete_after {
        rows.push(Line::default());
        rows.push(Line::styled(
            format!("!  Account scheduled for deletion on {date}."),
            theme::warn(),
        ));
    }
    rows
}

fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        .title(Span::styled(format!(" {title} "), theme::accent().bold()))
        .padding(ratatui::widgets::Padding::new(2, 2, 1, 0))
}

fn label(text: &str) -> Span<'static> {
    Span::styled(format!("{text:<11}"), theme::dim())
}

fn row<'a>(name: &str, value: Span<'a>) -> Line<'a> {
    Line::from(vec![label(name), value])
}

/// `████░░░░`: green, yellow from 75 %, red from 90 %.
fn meter(used: u64, total: u64, width: usize) -> [Span<'static>; 2] {
    let ratio = if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    };
    let filled = ((ratio * width as f64).round() as usize).min(width);
    let colour = if ratio >= 0.9 {
        theme::error()
    } else if ratio >= 0.75 {
        theme::warn()
    } else {
        theme::brand()
    };
    [
        Span::styled("█".repeat(filled), colour),
        Span::styled("░".repeat(width - filled), theme::border()),
    ]
}

/// `1536` → `1.5 KB`.
fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn centered(area: Rect, max_width: u16) -> Rect {
    let width = area.width.min(max_width);
    let inner = Rect {
        x: area.x + (area.width - width) / 2,
        width,
        ..area
    };
    inner.inner(ratatui::layout::Margin::new(1, 1))
}

/// Draws the traced logo, centred in its area.
struct MarkWidget<'a>(&'a Mark);

impl Widget for MarkWidget<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let mark = self.0;
        let left = area.x + area.width.saturating_sub(mark.width()) / 2;
        let top = area.y + area.height.saturating_sub(mark.height()) / 2;
        for (y, row) in mark.rows.iter().enumerate() {
            for (x, glyph) in row.iter().enumerate() {
                let (x, y) = (left + x as u16, top + y as u16);
                if x >= area.right() || y >= area.bottom() {
                    continue;
                }
                let mut style = Style::new();
                if let Some(fg) = glyph.fg {
                    style = style.fg(theme::rgb(fg));
                }
                if let Some(bg) = glyph.bg {
                    style = style.bg(theme::rgb(bg));
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(glyph.ch).set_style(style);
                }
            }
        }
    }
}

/// The traced logo sized for the header.
pub fn logo_blocks() -> LogoView {
    LogoView::Blocks(logo::mark(LOGO_COLUMNS, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app(connection: Connection) -> App {
        App::new(
            Launch {
                version: "v0.1.0".into(),
                server: "https://skillpouch.net".into(),
            },
            connection,
            logo_blocks(),
        )
    }

    fn account() -> Account {
        Account {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            computer: Some("laptop".into()),
            plan: "Supporter".into(),
            used_bytes: 1536,
            quota_bytes: 1_073_741_824,
            delete_after: None,
        }
    }

    fn screen(app: &mut App) -> String {
        let Ok(mut terminal) = Terminal::new(TestBackend::new(96, 26));
        let Ok(_) = terminal.draw(|frame| app.draw(frame));
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(96)
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press(app: &mut App, code: KeyCode) -> Option<Action> {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn a_new_computer_is_offered_login() {
        let mut app = app(Connection::NotConnected);
        let text = screen(&mut app);
        assert!(text.contains("SkillPouch"));
        assert!(text.contains("isn't connected yet"));
        assert!(text.contains("› Log in"));
        assert_eq!(press(&mut app, KeyCode::Enter), Some(Action::Login));
    }

    #[test]
    fn a_connected_computer_shows_its_account() {
        let mut app = app(Connection::Connected(account()));
        let text = screen(&mut app);
        for expected in [
            "Ada",
            "ada@example.com",
            "laptop",
            "Supporter",
            "1.5 KB of 1.0 GB",
        ] {
            assert!(text.contains(expected), "missing {expected}:\n{text}");
        }
        assert!(text.contains("› Refresh"));
    }

    #[test]
    fn arrows_move_through_the_actions_and_wrap() {
        let mut app = app(Connection::Connected(account()));
        assert_eq!(press(&mut app, KeyCode::Up), None);
        assert_eq!(press(&mut app, KeyCode::Enter), Some(Action::Quit));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Enter), Some(Action::Login));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(press(&mut app, KeyCode::Enter), Some(Action::Logout));
        assert_eq!(press(&mut app, KeyCode::Char('q')), Some(Action::Quit));
    }

    #[test]
    fn the_selection_stays_valid_when_the_actions_shrink() {
        let mut app = app(Connection::Connected(account()));
        press(&mut app, KeyCode::Up);
        app.set_connection(Connection::NotConnected);
        assert_eq!(press(&mut app, KeyCode::Enter), Some(Action::Quit));
    }

    #[test]
    fn failures_and_checks_are_shown() {
        let mut app = app(Connection::Failed("Server unreachable.".into()));
        let text = screen(&mut app);
        assert!(text.contains("Server unreachable."));
        assert!(text.contains("› Try again"));
        let mut app = super::tests::app(Connection::Checking);
        assert!(screen(&mut app).contains("Checking your account"));
    }

    #[test]
    fn storage_meter_changes_colour_as_it_fills() {
        assert_eq!(meter(0, 0, 4)[0].content, "");
        assert_eq!(meter(50, 100, 4)[0].content, "██");
        assert_eq!(meter(95, 100, 4)[0].style, theme::error());
    }
}
