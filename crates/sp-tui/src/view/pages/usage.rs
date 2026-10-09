//! Tokens of every computer: totals, every day since the first one as
//! columns across the page, and the last week, models and assistants side
//! by side.

use super::loaded;
use crate::app::App;
use crate::theme;
use crate::usage::{Period, Tokens, Usage, compact};
use crate::view::chart::{self, bar_line};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

/// Rows of columns: as tall as the page allows, within these.
const MIN_ROWS: u16 = 4;
const MAX_ROWS: u16 = 12;
/// Days in the "Last 7 days" list.
const WEEK: i64 = 7;
/// Rows per breakdown list.
const TOP: usize = 7;

pub fn usage(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let usage = match &dashboard.usage {
        Ok(usage) => usage,
        Err(reason) => {
            let lines = vec![
                Line::styled("Token usage isn't available.", theme::text()),
                Line::styled(reason.clone(), theme::meta()),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
            return;
        }
    };
    let [totals, _, recent, _, breakdown, _, split_line] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(MIN_ROWS + 3),
        Constraint::Length(1),
        Constraint::Length(TOP as u16 + 1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let columns = Layout::horizontal([Constraint::Length(18); 4]).split(totals);
    let figures = Period::ALL
        .iter()
        .map(|period| (period.label(), usage.total(*period).total()))
        .chain([("All time", usage.all_time.total())]);
    for ((label, value), column) in figures.zip(columns.iter()) {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(label, theme::meta()),
                Line::styled(compact(value), theme::strong()),
            ]),
            *column,
        );
    }

    let width = usize::from(recent.width);
    let days = chart::chart_days(usage, width);
    let rows = recent.height.saturating_sub(3).clamp(MIN_ROWS, MAX_ROWS);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("Last {days} days"), theme::strong()),
        Span::styled(peak(usage, days), theme::meta()),
    ])];
    lines.extend(chart::columns(usage, days, width, usize::from(rows)));
    frame.render_widget(Paragraph::new(lines), recent);

    let [week, _, models, _, assistants] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .areas(breakdown);
    list(frame, week, "Last 7 days", &last_week(usage));
    let from = usage.start(Period::Month);
    list(frame, models, "This month by model", &usage.by_model(from));
    list(
        frame,
        assistants,
        "This month by assistant",
        &usage.by_assistant(from),
    );
    frame.render_widget(
        Paragraph::new(kinds(&usage.total(Period::Month))).wrap(Wrap { trim: false }),
        split_line,
    );
}

/// Today, Yesterday, then `Wed 7` for the days before, newest first.
fn last_week(usage: &Usage) -> Vec<(String, u64)> {
    let per_day = usage.per_day();
    (0..WEEK)
        .map(|back| {
            let day = usage.today - time::Duration::days(back);
            let name = match back {
                0 => "Today".to_string(),
                1 => "Yesterday".to_string(),
                _ => format!("{:.3} {}", day.weekday().to_string(), day.day()),
            };
            (name, per_day.get(&day).copied().unwrap_or(0))
        })
        .collect()
}

/// `   peak 4.1M on Oct 3`
fn peak(usage: &Usage, days: usize) -> String {
    let from = usage.today - time::Duration::days(i64::try_from(days).unwrap_or(0) - 1);
    usage
        .per_day()
        .range(from..)
        .max_by_key(|(_, total)| **total)
        .filter(|(_, total)| **total > 0)
        .map_or(String::new(), |(day, total)| {
            format!(
                "   peak {} on {} {}",
                compact(*total),
                chart::short_month(day.month()),
                day.day()
            )
        })
}

fn list(frame: &mut Frame, area: Rect, title: &str, rows: &[(String, u64)]) {
    let mut lines = vec![Line::styled(title.to_string(), theme::strong())];
    if rows.is_empty() {
        lines.push(Line::styled("Nothing yet.", theme::meta()));
    }
    let max = rows.iter().map(|(_, value)| *value).max().unwrap_or(0);
    let width = usize::from(area.width);
    let name_width = rows
        .iter()
        .take(TOP)
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0)
        .min(width / 2)
        .max(6);
    let bar_width = width.saturating_sub(name_width + 2 + 2 + 6).clamp(4, 24);
    for (name, value) in rows.iter().take(TOP) {
        lines.push(bar_line(name, *value, max, name_width, bar_width));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// What this month's tokens were.
fn kinds(month: &Tokens) -> Line<'static> {
    let parts = [
        ("input", month.input),
        ("output", month.output),
        ("cache read", month.cache_read),
        ("cache write", month.cache_write),
    ];
    let mut spans = vec![Span::styled("This month  ", theme::meta())];
    for (index, (label, value)) in parts.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ·  ", theme::meta()));
        }
        spans.push(Span::styled(compact(value), theme::text()));
        spans.push(Span::styled(format!(" {label}"), theme::meta()));
    }
    Line::from(spans)
}
