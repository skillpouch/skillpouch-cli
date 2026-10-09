//! The mascot, the pouch this computer syncs, how much of the plan is used,
//! and a year of tokens.

use super::{loaded, sync};
use crate::app::App;
use crate::mascot::{self, Mood};
use crate::model::{Dashboard, SyncTone, SyncView};
use crate::theme;
use crate::usage::{Period, Usage, compact};
use crate::view::chart;
use crate::view::widgets::{bytes, field, meter, spinner};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

/// Longest the plan bars get.
const METER: usize = 28;
/// Rows the token section needs: heading, totals, gap, months, seven days.
const TOKENS_HEIGHT: u16 = 11;
/// The mascot gets this much room beside the details.
const MASCOT_GAP: u16 = 4;

pub fn overview(app: &App, frame: &mut Frame, area: Rect) {
    let Some(dashboard) = loaded(app, frame, area) else {
        return;
    };
    let details = details(dashboard, area.width, app.tick);
    let details_height = u16::try_from(details.len()).unwrap_or(u16::MAX);
    let with_mascot = area.height >= mascot::HEIGHT + 1 + TOKENS_HEIGHT
        && area.width >= mascot::WIDTH + MASCOT_GAP + 48;
    let top_height = if with_mascot {
        mascot::HEIGHT.max(details_height)
    } else {
        details_height
    };
    let [top, _, tokens] = Layout::vertical([
        Constraint::Length(top_height),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);

    let details_area = if with_mascot {
        let [figure, _, rest] = Layout::horizontal([
            Constraint::Length(mascot::WIDTH),
            Constraint::Length(MASCOT_GAP),
            Constraint::Fill(1),
        ])
        .areas(top);
        let pose = mascot::pose(app.tick, Mood::Idle);
        frame.render_widget(Paragraph::new(mascot::render(&pose)), figure);
        // Centred against the mascot.
        let offset = top_height.saturating_sub(details_height) / 2;
        Rect {
            y: rest.y + offset,
            height: rest.height - offset,
            ..rest
        }
    } else {
        top
    };
    frame.render_widget(
        Paragraph::new(details).wrap(Wrap { trim: false }),
        details_area,
    );
    token_section(dashboard, frame, tokens);
}

/// `● Synced 2 min ago`, with the reason under it when there is one.
fn sync_lines(view: &SyncView, tick: usize) -> Vec<Line<'static>> {
    let (mark, style) = match view.tone {
        SyncTone::Good => (Span::styled("● ", theme::success()), theme::text()),
        SyncTone::Busy => (spinner(tick), theme::text()),
        SyncTone::Warn => (Span::styled("● ", theme::warn()), theme::warn()),
        SyncTone::Bad => (Span::styled("● ", theme::error()), theme::error()),
    };
    let mut lines = vec![field(
        "Sync",
        vec![mark, Span::styled(view.text.clone(), style)],
    )];
    if let Some(detail) = &view.detail {
        lines.push(field("", vec![Span::styled(detail.clone(), theme::meta())]));
    }
    lines
}

fn details(dashboard: &Dashboard, width: u16, tick: usize) -> Vec<Line<'static>> {
    let account = &dashboard.account;
    let first = account
        .name
        .split_whitespace()
        .next()
        .unwrap_or(&account.name)
        .to_string();
    // The label column, the counts after the bar and some air.
    let bar = usize::from(width)
        .saturating_sub(mascot::WIDTH as usize + 14 + 22)
        .clamp(8, METER);

    let mut lines = vec![
        Line::styled(format!("Hi, {first}"), theme::strong()),
        Line::styled(account.email.clone(), theme::meta()),
        Line::default(),
    ];
    match dashboard.active_pouch() {
        Some(pouch) => {
            lines.push(field(
                "Active pouch",
                vec![Span::styled(pouch.label(), theme::accent())],
            ));
            match &dashboard.sync {
                Some(view) => lines.extend(sync_lines(view, tick)),
                None => {
                    let status = dashboard
                        .this_computer()
                        .map_or(Span::styled("not synced yet", theme::meta()), |this| {
                            sync(pouch, this)
                        });
                    lines.push(field("", vec![status]));
                }
            }
            if let Some(first) = &dashboard.first_sync {
                lines.push(field(
                    "First sync",
                    vec![Span::styled(first.clone(), theme::text())],
                ));
            }
        }
        None => {
            lines.push(field(
                "Active pouch",
                vec![Span::styled("None", theme::text())],
            ));
            lines.push(field(
                "",
                vec![Span::styled(
                    "Pick the one this computer syncs on the Pouches tab",
                    theme::meta(),
                )],
            ));
        }
    }

    let used = dashboard.live_pouches() as u64;
    let mut pouches = meter(used, account.pouch_limit, bar);
    pouches.push(Span::styled(
        format!("  {used} of {}", account.pouch_limit),
        theme::muted(),
    ));
    lines.push(field("Pouches", pouches));

    let mut storage = meter(account.used_bytes, account.quota_bytes, bar);
    storage.push(Span::styled(
        format!(
            "  {} of {}",
            bytes(account.used_bytes),
            bytes(account.quota_bytes)
        ),
        theme::muted(),
    ));
    lines.push(field("Storage", storage));
    lines.push(field(
        "Plan",
        vec![Span::styled(account.plan.clone(), theme::text())],
    ));
    if let Some(date) = &account.delete_after {
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("Your account will be deleted on {date}. Cancel it in the web app."),
            theme::warn(),
        ));
    }
    lines
}

fn token_section(dashboard: &Dashboard, frame: &mut Frame, area: Rect) {
    let [heading, totals, _, chart] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::styled("Tokens", theme::strong())),
        heading,
    );
    let usage = match &dashboard.usage {
        Ok(usage) => usage,
        Err(reason) => {
            frame.render_widget(
                Paragraph::new(Line::styled(reason.clone(), theme::meta()))
                    .wrap(Wrap { trim: false }),
                totals,
            );
            return;
        }
    };
    frame.render_widget(Paragraph::new(chart::legend()).right_aligned(), heading);
    frame.render_widget(Paragraph::new(total_line(usage)), totals);
    let width = usize::from(chart.width);
    frame.render_widget(Paragraph::new(chart::grid(usage, width)), chart);
}

/// `Today 1.2M    This week 8.4M    This month 30M    All time 120M`
fn total_line(usage: &Usage) -> Line<'static> {
    let mut spans = Vec::new();
    let totals = Period::ALL
        .iter()
        .map(|period| (period.label(), usage.total(*period).total()))
        .chain([("All time", usage.all_time.total())]);
    for (index, (label, value)) in totals.enumerate() {
        if index > 0 {
            spans.push(Span::raw("     "));
        }
        spans.push(Span::styled(format!("{label} "), theme::meta()));
        spans.push(Span::styled(compact(value), theme::strong()));
    }
    Line::from(spans)
}
