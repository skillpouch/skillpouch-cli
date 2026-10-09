//! Every computer on the account and the pouch each one syncs.

use super::{loaded, render_table, split, synced_by, table};
use crate::app::{App, Tab};
use crate::theme;
use crate::view::widgets::{capitalised, field, key_storage, list_field, state_style};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Wrap};

pub fn computers(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let rows: Vec<Row> = dashboard
        .computers
        .iter()
        .map(|computer| {
            let mut name = vec![Span::styled(computer.name.as_str(), theme::text())];
            if computer.current {
                name.push(Span::styled("  this computer", theme::meta()));
            }
            Row::new(vec![
                Cell::from(Line::from(name)),
                Cell::from(Span::styled(
                    synced_by(dashboard, computer).unwrap_or_else(|| "Nothing".into()),
                    theme::muted(),
                )),
                Cell::from(Span::styled(
                    computer.last_seen.clone().unwrap_or_else(|| "Never".into()),
                    theme::muted(),
                )),
                Cell::from(Span::styled(
                    capitalised(&computer.state),
                    state_style(&computer.state),
                )),
            ])
        })
        .collect();
    let (top, bottom) = split(area, rows.len());
    render_table(
        frame,
        top,
        table(
            rows,
            &[
                Constraint::Fill(1),
                Constraint::Fill(1),
                Constraint::Length(11),
                Constraint::Length(9),
            ],
            &["Computer", "Syncs", "Last seen", "State"],
        ),
        app.cursor(Tab::Computers),
    );

    let Some(computer) = dashboard.computers.get(app.cursor(Tab::Computers)) else {
        return;
    };
    let text = |value: String| vec![Span::styled(value, theme::text())];
    let mut lines = vec![
        Line::styled(computer.name.clone(), theme::strong()),
        Line::default(),
        field("System", text(computer.platform.clone())),
        field("CLI", text(format!("v{}", computer.cli_version))),
        field(
            "Keys",
            text(
                computer
                    .key_storage
                    .as_deref()
                    .map_or("Unknown", key_storage)
                    .into(),
            ),
        ),
        field(
            "Connected",
            text(
                computer
                    .connected
                    .clone()
                    .unwrap_or_else(|| "Unknown".into()),
            ),
        ),
    ];
    lines.extend(list_field("Assistants", &computer.assistants));
    lines.push(Line::default());
    lines.push(Line::styled(
        match dashboard.browsers {
            0 => "No trusted browsers.".to_string(),
            1 => "Your account also has 1 trusted browser.".to_string(),
            n => format!("Your account also has {n} trusted browsers."),
        },
        theme::meta(),
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), bottom);
}
