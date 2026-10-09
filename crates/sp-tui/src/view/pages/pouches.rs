//! Every pouch, the one this computer syncs marked; enter switches to the
//! selected one.

use super::{loaded, render_table, split, sync, table};
use crate::app::{App, Tab};
use crate::model::Pouch;
use crate::theme;
use crate::view::widgets::{capitalised, field, state_style};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Wrap};

fn items(pouch: &Pouch) -> String {
    if pouch.more {
        format!("{}+", pouch.items)
    } else {
        pouch.items.to_string()
    }
}

pub fn pouches(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let active = dashboard.active_pouch().map(|pouch| pouch.id.as_str());
    let rows: Vec<Row> = dashboard
        .pouches
        .iter()
        .map(|pouch| {
            let here = active == Some(pouch.id.as_str());
            let name = if here {
                Line::from(vec![
                    Span::styled("● ", theme::brand()),
                    Span::styled(pouch.label(), theme::strong()),
                ])
            } else {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(pouch.label(), theme::text()),
                ])
            };
            Row::new(vec![
                Cell::from(name),
                Cell::from(items(pouch)),
                Cell::from(Span::styled(
                    pouch.updated.clone().unwrap_or_else(|| "Never".into()),
                    theme::muted(),
                )),
                Cell::from(Span::styled(
                    capitalised(&pouch.state),
                    state_style(&pouch.state),
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
                Constraint::Length(6),
                Constraint::Length(12),
                Constraint::Length(8),
            ],
            &["  Pouch", "Items", "Updated", "State"],
        ),
        app.cursor(Tab::Pouches),
    );

    let Some(pouch) = dashboard.pouches.get(app.cursor(Tab::Pouches)) else {
        let line = Line::styled("No pouches yet. Create one in the web app.", theme::meta());
        frame.render_widget(Paragraph::new(line), bottom);
        return;
    };
    let mut lines = vec![Line::styled(pouch.label(), theme::strong())];
    if let Some(description) = pouch.description.as_deref().filter(|d| !d.is_empty()) {
        lines.push(Line::styled(description.to_string(), theme::muted()));
    }
    lines.push(Line::default());
    lines.push(if active == Some(pouch.id.as_str()) {
        Line::styled("This computer syncs this pouch.", theme::success())
    } else if pouch.usable() {
        Line::from(vec![
            Span::styled("enter", theme::text()),
            Span::styled(
                " to sync this pouch on this computer instead. Each computer syncs one pouch.",
                theme::meta(),
            ),
        ])
    } else {
        Line::styled(
            format!(
                "{} pouches don't sync. Manage it in the web app.",
                capitalised(&pouch.state)
            ),
            theme::warn(),
        )
    });

    let syncing: Vec<_> = dashboard
        .computers
        .iter()
        .filter(|computer| computer.active_pouch.as_deref() == Some(pouch.id.as_str()))
        .collect();
    if !syncing.is_empty() {
        lines.push(Line::default());
        lines.push(Line::styled("Synced on", theme::meta()));
        for computer in syncing {
            lines.push(field(&computer.name, vec![sync(pouch, computer)]));
        }
    }
    if let Some(reason) = &dashboard.names_missing {
        lines.push(Line::default());
        lines.push(Line::styled(reason.clone(), theme::meta()));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), bottom);
}
