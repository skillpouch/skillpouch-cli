//! Who is signed in, and what can be done about it.

use super::{loaded, render_table, table};
use crate::app::{AccountAction, App, Tab};
use crate::theme;
use crate::view::widgets::field;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Wrap};

pub fn account(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let account = &dashboard.account;
    let mut lines = vec![
        field(
            "Name",
            vec![Span::styled(account.name.as_str(), theme::text())],
        ),
        field(
            "Email",
            vec![Span::styled(account.email.as_str(), theme::text())],
        ),
        field(
            "Plan",
            vec![Span::styled(account.plan.as_str(), theme::text())],
        ),
        field(
            "Server",
            vec![Span::styled(app.launch.server.as_str(), theme::muted())],
        ),
    ];
    if let Some(date) = &account.delete_after {
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("Your account will be deleted on {date}. Cancel it in the web app."),
            theme::warn(),
        ));
    }
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let [details, _, actions] = Layout::vertical([
        Constraint::Length(height),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), details);

    let rows: Vec<Row> = AccountAction::ALL
        .iter()
        .map(|action| {
            let style = if *action == AccountAction::Logout {
                theme::error()
            } else {
                theme::text()
            };
            Row::new(vec![
                Cell::from(Span::styled(action.label(), style)),
                Cell::from(Span::styled(action.detail(), theme::meta())),
            ])
        })
        .collect();
    render_table(
        frame,
        actions,
        table(
            rows,
            &[Constraint::Length(28), Constraint::Fill(1)],
            &["Actions", ""],
        ),
        app.cursor(Tab::Account),
    );
}
