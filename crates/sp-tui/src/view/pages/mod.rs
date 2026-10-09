//! The six tabs. Plain text and tables, no boxes: headings, label columns
//! and a highlighted row are enough.

mod account;
mod choice;
mod computers;
mod overview;
mod pouches;
mod usage;

pub use account::account;
pub use choice::choice;
pub use computers::computers;
pub use overview::overview;
pub use pouches::pouches;
pub use usage::usage;

use super::widgets::spinner;
use crate::app::{App, Status};
use crate::model::{Computer, Dashboard, Pouch};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState, Wrap};

/// The dashboard, or a line saying why there isn't one yet.
fn loaded<'a>(app: &'a App, frame: &mut Frame, area: Rect) -> Option<&'a Dashboard> {
    let lines = match &app.status {
        Status::Ready(dashboard) => return Some(dashboard),
        Status::Loading | Status::SignedOut => vec![Line::from(vec![
            spinner(app.tick),
            Span::styled("  Loading your account", theme::muted()),
        ])],
        Status::Failed(message) => vec![
            Line::styled("Couldn't load your account.", theme::text()),
            Line::styled(message.as_str(), theme::error()),
            Line::default(),
            Line::from(vec![
                Span::styled("r", theme::text()),
                Span::styled(" to try again", theme::meta()),
            ]),
        ],
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    None
}

fn table<'a>(rows: Vec<Row<'a>>, widths: &[Constraint], head: &[&'static str]) -> Table<'a> {
    Table::new(rows, widths.to_vec())
        .header(
            Row::new(head.iter().map(|c| Cell::from(*c)))
                .style(theme::meta())
                .bottom_margin(1),
        )
        .column_spacing(3)
        .row_highlight_style(theme::selected())
        .highlight_spacing(HighlightSpacing::Never)
}

fn render_table(frame: &mut Frame, area: Rect, table: Table, selected: usize) {
    let mut state = TableState::new().with_selected(Some(selected));
    frame.render_stateful_widget(table, area, &mut state);
}

/// The table sized to its rows, and what's left under it for details.
fn split(area: Rect, rows: usize) -> (Rect, Rect) {
    let height = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(2);
    let [top, _, bottom] = Layout::vertical([
        Constraint::Length(height),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(area);
    (top, bottom)
}

/// How far `computer` is with `pouch`.
fn sync(pouch: &Pouch, computer: &Computer) -> Span<'static> {
    let acked = computer
        .cursors
        .iter()
        .find(|(id, _)| id == &pouch.id)
        .map(|(_, seq)| *seq);
    match acked {
        Some(seq) if seq >= pouch.event_seq => Span::styled("up to date", theme::success()),
        Some(seq) => {
            let behind = pouch.event_seq - seq;
            let noun = if behind == 1 { "change" } else { "changes" };
            Span::styled(format!("{behind} {noun} behind"), theme::warn())
        }
        None => Span::styled("not synced yet", theme::meta()),
    }
}

/// The name of the pouch `computer` syncs.
fn synced_by(dashboard: &Dashboard, computer: &Computer) -> Option<String> {
    let id = computer.active_pouch.as_deref()?;
    Some(
        dashboard
            .pouch(id)
            .map_or("A deleted pouch".into(), Pouch::label),
    )
}
