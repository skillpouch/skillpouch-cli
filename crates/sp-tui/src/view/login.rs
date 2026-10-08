//! The login screen: the pouch, the name, and one thing to do. Logging in
//! happens right here: the link, the code to compare, then waiting.

use super::widgets::{centered, keys, spinner};
use crate::app::{App, Login};
use crate::logo::{MARK, MARK_HEIGHT, MARK_WIDTH};
use crate::model::LoginStep;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

const WIDTH: u16 = 64;

pub fn draw(app: &App, login: &Login, frame: &mut Frame, area: Rect) {
    let body = body(app, login);
    let body_height = u16::try_from(body.len()).unwrap_or(u16::MAX);
    let with_mark = area.height >= MARK_HEIGHT + body_height + 6;
    let height = body_height + if with_mark { MARK_HEIGHT + 2 } else { 0 };
    let column = centered(area, WIDTH.min(area.width.saturating_sub(4)), height);

    let text = if with_mark {
        let [mark, _, text] = Layout::vertical([
            Constraint::Length(MARK_HEIGHT),
            Constraint::Length(2),
            Constraint::Fill(1),
        ])
        .areas(column);
        let mark = centered(mark, MARK_WIDTH, MARK_HEIGHT);
        let lines: Vec<Line> = MARK
            .iter()
            .map(|row| Line::styled(*row, theme::brand()))
            .collect();
        frame.render_widget(Paragraph::new(lines), mark);
        text
    } else {
        column
    };
    frame.render_widget(
        Paragraph::new(body).centered().wrap(Wrap { trim: false }),
        text,
    );

    let [_, footer] = Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]).areas(area);
    frame.render_widget(Paragraph::new(footer_keys(app, login)).centered(), footer);
}

fn body<'a>(app: &'a App, login: &'a Login) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::styled("SkillPouch", theme::strong()),
        Line::styled(
            "Your skills, agents and MCP servers on every computer",
            theme::meta(),
        ),
        Line::default(),
        Line::default(),
    ];
    match &login.step {
        None => {
            if app.signed_in() {
                lines.push(Line::styled(
                    "Log in again to give this computer new keys.",
                    theme::text(),
                ));
                lines.push(Line::styled(
                    "The current ones keep working until you approve.",
                    theme::meta(),
                ));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("Connect ", theme::text()),
                    Span::styled(app.launch.hostname.as_str(), theme::strong()),
                    Span::styled(" to your account.", theme::text()),
                ]));
                lines.push(Line::styled(
                    "You approve it in a browser where you're signed in.",
                    theme::meta(),
                ));
            }
            lines.push(Line::default());
            lines.push(Line::from(vec![
                Span::styled("Press ", theme::meta()),
                Span::styled("enter", theme::accent().add_modifier(Modifier::BOLD)),
                Span::styled(" to log in", theme::meta()),
            ]));
        }
        Some(LoginStep::Starting | LoginStep::Approved | LoginStep::Cancelled) => {
            lines.push(Line::from(vec![
                spinner(app.tick),
                Span::styled("  Creating this computer's keys", theme::muted()),
            ]));
        }
        Some(LoginStep::Waiting {
            url, code, opened, ..
        }) => {
            let elapsed = login.since.elapsed().as_secs();
            lines.push(Line::styled(
                "Approve this computer in your browser",
                theme::text(),
            ));
            lines.push(Line::styled(
                url.as_str(),
                theme::accent().add_modifier(Modifier::UNDERLINED),
            ));
            lines.push(Line::styled(
                if *opened {
                    "Opened it for you"
                } else {
                    "Open it with o"
                },
                theme::meta(),
            ));
            lines.push(Line::default());
            lines.push(Line::styled(
                "and check that it shows this code",
                theme::text(),
            ));
            lines.push(Line::default());
            lines.push(Line::styled(
                code.replace('-', "  "),
                theme::accent().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::default());
            lines.push(Line::default());
            lines.push(Line::from(vec![
                spinner(app.tick),
                Span::styled("  Waiting for approval", theme::muted()),
                Span::styled(
                    format!("  {}:{:02}", elapsed / 60, elapsed % 60),
                    theme::meta(),
                ),
            ]));
        }
        Some(LoginStep::Failed(message)) => {
            lines.push(Line::styled(message.as_str(), theme::error()));
            lines.push(Line::default());
            lines.push(Line::from(vec![
                Span::styled("Press ", theme::meta()),
                Span::styled("enter", theme::accent().add_modifier(Modifier::BOLD)),
                Span::styled(" to try again", theme::meta()),
            ]));
        }
    }
    lines
}

fn footer_keys(app: &App, login: &Login) -> Line<'static> {
    let back = if app.signed_in() { "back" } else { "cancel" };
    match &login.step {
        None if app.signed_in() => keys(&[("enter", "log in"), ("esc", "back")]),
        None => keys(&[("enter", "log in"), ("q", "quit")]),
        Some(LoginStep::Waiting { .. }) => keys(&[("o", "open link"), ("esc", back)]),
        Some(LoginStep::Failed(_)) => keys(&[("enter", "try again"), ("q", "quit")]),
        Some(_) => keys(&[("esc", back)]),
    }
}
