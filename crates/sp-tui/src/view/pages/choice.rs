//! What this computer syncs: its assistants, and what goes into them.

use super::{loaded, table};
use crate::app::{App, Tab};
use crate::model::ChoiceRow;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, TableState};

pub fn choice(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let choice = &dashboard.choice;
    let cursor = app.cursor(Tab::Sync);
    let height = |rows: usize| u16::try_from(rows.max(1) + 2).unwrap_or(u16::MAX);
    let [intro, assistants, _, kinds] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(height(choice.assistants.len())),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "Turned off here: SkillPouch leaves it alone and its files stay as they are. Other computers choose for themselves.",
            theme::meta(),
        )),
        intro,
    );
    let assistants_count = choice.assistants.len();
    if choice.assistants.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                "No assistants found on this computer.",
                theme::meta(),
            )),
            assistants,
        );
    } else {
        draw(
            frame,
            assistants,
            &choice.assistants,
            "Assistant",
            (cursor < assistants_count).then_some(cursor),
        );
    }
    draw(
        frame,
        kinds,
        &choice.kinds,
        "Synced into them",
        cursor.checked_sub(assistants_count),
    );
}

fn draw(
    frame: &mut Frame,
    area: Rect,
    rows: &[ChoiceRow],
    head: &'static str,
    selected: Option<usize>,
) {
    let rows: Vec<Row> = rows
        .iter()
        .map(|row| {
            let (mark, style) = if row.on {
                ("● on", theme::accent())
            } else {
                ("○ off", theme::meta())
            };
            Row::new(vec![
                Cell::from(Span::styled(mark, style)),
                Cell::from(Span::styled(row.name.as_str(), theme::text())),
                Cell::from(Span::styled(
                    row.note.clone().unwrap_or_default(),
                    theme::meta(),
                )),
            ])
        })
        .collect();
    let table = table(
        rows,
        &[
            Constraint::Length(6),
            Constraint::Length(22),
            Constraint::Fill(1),
        ],
        &["", head, ""],
    );
    let mut state = TableState::new().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut state);
}
