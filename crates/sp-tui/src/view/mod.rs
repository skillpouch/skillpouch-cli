//! Drawing. Signed out (or logging in again): only the login screen. Being
//! set up: only the setup screen.
//! Signed in: a header with the tabs, the open tab, and a footer with keys
//! and notices.

mod chart;
mod login;
mod pages;
mod setup;
mod widgets;

use crate::app::{App, Status, Tab};
use crate::model::Tone;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};
use widgets::{centered, keys, spinner};

const BRAND: &str = "SkillPouch";
/// Space between the name and the first tab, and between tabs; narrow
/// terminals get the smaller gap.
const TAB_GAP: usize = 4;
const TAB_GAP_NARROW: usize = 2;

pub fn draw(app: &App, frame: &mut Frame) {
    let area = frame.area();
    draw_screen(app, frame, area);
    if let Some(version) = &app.restart {
        draw_restart(app, version, frame, area);
    }
}

fn draw_screen(app: &App, frame: &mut Frame, area: Rect) {
    if let Some(login) = &app.login {
        login::draw(app, login, frame, area);
        draw_notice(app, frame, area);
        return;
    }
    if let (Some(setup), Some(dashboard)) = (&app.setup, app.dashboard()) {
        setup::draw(app, setup, dashboard, frame, area);
        draw_notice(app, frame, area);
        return;
    }
    let [header, _, body, _, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    draw_header(app, frame, header);
    let body = body.inner(Margin::new(2, 0));
    match app.tab {
        Tab::Overview => pages::overview(app, frame, body),
        Tab::Pouches => pages::pouches(app, frame, body),
        Tab::Usage => pages::usage(app, frame, body),
        Tab::Computers => pages::computers(app, frame, body),
        Tab::Sync => pages::choice(app, frame, body),
        Tab::Account => pages::account(app, frame, body),
    }
    draw_footer(app, frame, footer);
    if app.confirm_logout {
        draw_logout(frame, area);
    }
}

fn draw_header(app: &App, frame: &mut Frame, area: Rect) {
    let [top, rule] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    let top = top.inner(Margin::new(2, 0));

    let labels: usize = Tab::ALL.iter().map(|tab| tab.label().len()).sum();
    let gap = if usize::from(top.width) >= BRAND.len() + labels + TAB_GAP * Tab::ALL.len() {
        TAB_GAP
    } else {
        TAB_GAP_NARROW
    };
    let mut spans = vec![Span::styled(
        BRAND,
        theme::brand().add_modifier(Modifier::BOLD),
    )];
    let mut x = BRAND.len();
    let mut underline = (0, 0);
    for tab in Tab::ALL {
        spans.push(Span::raw(" ".repeat(gap)));
        x += gap;
        let active = tab == app.tab;
        if active {
            underline = (x, tab.label().len());
        }
        spans.push(Span::styled(
            tab.label(),
            if active {
                theme::strong()
            } else {
                theme::meta()
            },
        ));
        x += tab.label().len();
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), top);

    let mut right = match &app.status {
        Status::Ready(dashboard) => vec![
            Span::styled(dashboard.account.name.as_str(), theme::muted()),
            Span::styled("  ·  ", theme::meta()),
            Span::styled(dashboard.account.plan.as_str(), theme::accent()),
        ],
        Status::Loading => vec![Span::styled("Loading", theme::meta())],
        Status::Failed(_) => vec![Span::styled("Offline", theme::error())],
        Status::SignedOut => vec![],
    };
    if app.refreshing || matches!(app.status, Status::Loading) {
        right.insert(0, Span::raw("  "));
        right.insert(0, spinner(app.tick));
    }
    let right = Line::from(right);
    if usize::from(top.width) >= x + right.width() + TAB_GAP {
        frame.render_widget(Paragraph::new(right).right_aligned(), top);
    }

    // A thin rule, thicker and purple under the open tab.
    let width = usize::from(rule.width);
    let start = (underline.0 + 2).min(width);
    let end = (start + underline.1).min(width);
    let rule_line = Line::from(vec![
        Span::styled("─".repeat(start), Style::new().fg(theme::BORDER)),
        Span::styled("━".repeat(end - start), theme::brand()),
        Span::styled("─".repeat(width - end), Style::new().fg(theme::BORDER)),
    ]);
    frame.render_widget(Paragraph::new(rule_line), rule);
}

fn draw_footer(app: &App, frame: &mut Frame, area: Rect) {
    let area = area.inner(Margin::new(2, 0));
    let mut pairs = vec![("←→", "switch")];
    if matches!(app.tab, Tab::Pouches | Tab::Computers | Tab::Account) {
        pairs.push(("↑↓", "move"));
    }
    match app.tab {
        Tab::Pouches => pairs.push(("enter", "sync here")),
        Tab::Sync => pairs.push(("enter", "on / off")),
        Tab::Account => pairs.push(("enter", "select")),
        _ => {}
    }
    if app.conflicts_path().is_some() {
        pairs.push(("c", "resolve conflicts"));
    }
    pairs.extend([("o", "open in browser"), ("r", "refresh"), ("q", "quit")]);
    frame.render_widget(Paragraph::new(keys(&pairs)), area);
    draw_notice(app, frame, area);
}

/// The latest notice, right-aligned on the bottom row; without one, how
/// updating SkillPouch goes.
fn draw_notice(app: &App, frame: &mut Frame, area: Rect) {
    let line = match (app.notice(), app.upgrade_status()) {
        (Some(notice), _) => Line::styled(
            notice.text.as_str(),
            match notice.tone {
                Tone::Success => theme::success(),
                Tone::Info => theme::muted(),
                Tone::Error => theme::error(),
            },
        ),
        (None, Some(status)) => Line::styled(status, theme::meta()),
        (None, None) => return,
    };
    let row = Rect {
        y: area.bottom().saturating_sub(1),
        height: 1,
        ..area
    }
    .inner(Margin::new(2, 0));
    let width = u16::try_from(line.width()).unwrap_or(u16::MAX);
    frame.render_widget(
        Clear,
        Rect {
            x: row.right().saturating_sub(width),
            width: width.min(row.width),
            ..row
        },
    );
    frame.render_widget(Paragraph::new(line).right_aligned(), row);
}

fn draw_restart(app: &App, version: &str, frame: &mut Frame, area: Rect) {
    let rect = centered(area, 60, 8);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER_STRONG))
        .padding(Padding::new(2, 2, 1, 0));
    let lines = vec![
        Line::styled(
            format!("SkillPouch updated to {version} — restart now?"),
            theme::strong(),
        ),
        Line::styled(
            format!("Later keeps {} running until you quit.", app.launch.version),
            theme::meta(),
        ),
        Line::default(),
        keys(&[("enter", "restart"), ("esc", "later")]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

fn draw_logout(frame: &mut Frame, area: Rect) {
    let rect = centered(area, 60, 8);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER_STRONG))
        .padding(Padding::new(2, 2, 1, 0));
    let lines = vec![
        Line::styled("Log out of this computer?", theme::strong()),
        Line::styled(
            "Its keys are deleted. It stays on your Devices page.",
            theme::meta(),
        ),
        Line::default(),
        keys(&[("y", "log out"), ("n", "cancel")]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::tests::{launch, ready};
    use crate::model::{LoginStep, Update, Upgrade};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    pub fn screen(app: &App, width: u16, height: u16) -> String {
        let Ok(mut terminal) = Terminal::new(TestBackend::new(width, height));
        let _ = terminal.draw(|frame| draw(app, frame));
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn contains_all(text: &str, expected: &[&str]) {
        for item in expected {
            assert!(text.contains(item), "missing {item:?}:\n{text}");
        }
    }

    #[test]
    fn signed_out_shows_only_the_login_screen() {
        let app = App::new(launch(), false);
        let text = screen(&app, 100, 32);
        contains_all(
            &text,
            &[
                "▄█▀█▄",
                "SkillPouch",
                "Connect laptop",
                "enter to log in",
                "q quit",
            ],
        );
        for absent in ["Overview", "Pouches", "Tokens"] {
            assert!(!text.contains(absent), "{absent} shown:\n{text}");
        }
    }

    #[test]
    fn the_login_screen_shows_the_link_and_the_code() {
        let mut app = App::new(launch(), false);
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(screen(&app, 100, 32).contains("Creating this computer's keys"));
        app.apply(Update::Login(LoginStep::Waiting {
            url: "https://app.skillpouch.net/cli/authorize?request=abc".into(),
            code: "2605-3825".into(),
            computer: "laptop".into(),
            opened: true,
        }));
        contains_all(
            &screen(&app, 100, 32),
            &[
                "https://app.skillpouch.net/cli/authorize?request=abc",
                "2605  3825",
                "Waiting for approval",
                "esc cancel",
            ],
        );
        // Short terminals drop the pouch, not the code.
        let short = screen(&app, 100, 20);
        assert!(!short.contains("▄█▀█▄") && short.contains("2605  3825"));
    }

    #[test]
    fn the_overview_shows_how_syncing_goes() {
        let mut app = ready();
        app.apply(Update::Sync(Some(crate::model::SyncView {
            tone: crate::model::SyncTone::Warn,
            text: "Offline".into(),
            detail: Some("Can't reach the server · last synced 5 min ago".into()),
            conflicts: None,
        })));
        let text = screen(&app, 120, 40);
        contains_all(&text, &["Sync", "Offline", "last synced 5 min ago"]);
        // Not set up (or signed out): no sync line at all.
        app.apply(Update::Sync(None));
        assert!(!screen(&app, 120, 40).contains("Offline"));
    }

    #[test]
    fn conflicts_open_the_web_app_with_c() {
        let mut app = ready();
        app.apply(Update::Sync(Some(crate::model::SyncView {
            tone: crate::model::SyncTone::Warn,
            text: "Synced just now".into(),
            detail: Some(
                "1 item changed in two places: press c to pick versions in the web app".into(),
            ),
            conflicts: Some("/app/p/p1/conflicts".into()),
        })));
        contains_all(
            &screen(&app, 120, 40),
            &["changed in two places", "c resolve conflicts"],
        );
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(matches!(
            app.on_key(key),
            crate::app::Outcome::Send(crate::model::Request::Open(url)) if url == "https://app.skillpouch.net/app/p/p1/conflicts"
        ));
    }

    #[test]
    fn every_tab_draws() {
        let mut app = ready();
        let expected: [&[&str]; 6] = [
            &[
                "Hi, Ada",
                "Work stuff",
                "up to date",
                "2 of 3",
                "1.5 KB of 1.0 GB",
                "Tokens",
                "This week 3.5K",
                "All time 15.5K",
                "Mon ",
            ],
            &[
                "● Work stuff",
                "Pouch 2",
                "Locked",
                "Skills for the day job",
                "This computer syncs this pouch.",
                "laptop        up to date",
            ],
            &[
                "This week",
                "3.5K",
                "Last 14 days",
                "peak 8.0K on Sep 30",
                "claude-sonnet",
                "This month by assistant",
                "3.8K output",
            ],
            &[
                "Assistant",
                "Claude Code",
                "only read: part of a git checkout",
                "Synced into them",
                "● on",
                "Hooks",
            ],
            &[
                "desktop",
                "Work stuff",
                "2 days ago",
                "this computer",
                "Linux x86_64",
                "1 trusted browser",
            ],
            &["ada@example.com", "Manage plan and billing", "Log out"],
        ];
        for (index, expected) in expected.iter().enumerate() {
            app.tab = Tab::ALL[index];
            let text = screen(&app, 110, 34);
            contains_all(&text, expected);
            contains_all(&text, &["SkillPouch", "Ada Lovelace", "Supporter"]);
        }
    }

    #[test]
    fn short_terminals_drop_the_mascot_and_keep_the_tokens() {
        let app = ready();
        let tall = screen(&app, 110, 34);
        let short = screen(&app, 110, 26);
        assert!(tall.contains("▄"), "{tall}");
        assert!(
            short.contains("Tokens") && short.contains("Active pouch"),
            "{short}"
        );
    }

    #[test]
    fn missing_usage_and_names_say_why() {
        let mut app = ready();
        let mut dashboard = crate::app::tests::dashboard();
        dashboard.usage = Some(Err("Couldn't reach the server.".into()));
        dashboard.names_missing = Some("Log in again to see pouch names.".into());
        app.apply(Update::Loaded(Box::new(dashboard)));
        assert!(screen(&app, 110, 34).contains("Couldn't reach the server."));
        app.tab = Tab::Usage;
        assert!(screen(&app, 110, 34).contains("Token usage isn't available."));
        app.tab = Tab::Pouches;
        assert!(screen(&app, 110, 34).contains("Log in again to see pouch names."));
    }

    #[test]
    fn narrow_terminals_drop_the_account_name_not_the_tabs() {
        let app = ready();
        let text = screen(&app, 72, 30);
        assert!(text.contains("Account") && !text.contains("Ada Lovelace  ·"));
    }

    #[test]
    fn the_footer_says_how_updating_goes() {
        let mut app = ready();
        let quiet = screen(&app, 110, 34);
        assert!(!quiet.contains("update"), "{quiet}");
        app.apply(Update::Upgrade(Upgrade::Checking));
        let checking = screen(&app, 110, 34);
        let footer = checking.lines().last().unwrap_or_default();
        assert!(
            footer.contains("q quit") && footer.contains("Checking for updates…"),
            "{checking}"
        );
        app.apply(Update::Upgrade(Upgrade::Installing("0.2.3".into())));
        let installing = screen(&app, 110, 34);
        let footer = installing.lines().last().unwrap_or_default();
        assert!(
            footer.contains("Updating SkillPouch to 0.2.3…"),
            "{installing}"
        );
        app.apply(Update::Upgrade(Upgrade::Failed {
            reason: "SkillPouch is in use".into(),
            hint: Some("Quit, then run `npm i -g skillpouch`.".into()),
        }));
        let failed = screen(&app, 110, 34);
        let footer = failed.lines().last().unwrap_or_default();
        assert!(
            footer.contains(
                "Couldn't update: SkillPouch is in use. Quit, then run `npm i -g skillpouch`."
            ) && !footer.contains("Updating"),
            "{failed}"
        );
        // Also behind the login screen.
        let mut signed_out = App::new(launch(), false);
        signed_out.apply(Update::Upgrade(Upgrade::Checking));
        assert!(screen(&signed_out, 100, 32).contains("Checking for updates…"));
    }

    #[test]
    fn a_finished_update_asks_to_restart() {
        let mut app = ready();
        app.apply(Update::Upgrade(Upgrade::Installing("0.2.3".into())));
        app.apply(Update::Upgrade(Upgrade::Installed("0.2.3".into())));
        let text = screen(&app, 110, 34);
        contains_all(
            &text,
            &[
                "SkillPouch updated to 0.2.3 — restart now?",
                "Later keeps v0.1.0 running until you quit.",
                "enter restart",
                "esc later",
                "Hi, Ada",
            ],
        );
        assert!(!text.contains("Updating SkillPouch"), "{text}");
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!screen(&app, 110, 34).contains("restart now?"));
    }

    #[test]
    fn logout_asks_in_a_dialog() {
        let mut app = ready();
        app.confirm_logout = true;
        contains_all(
            &screen(&app, 110, 34),
            &["Log out of this computer?", "y log out"],
        );
    }
}
