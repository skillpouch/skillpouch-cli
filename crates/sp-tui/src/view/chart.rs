//! Token charts: the year as a grid of days (one column per week, darker
//! for busier days) and recent days as bars.

use crate::theme;
use crate::usage::{ROW_LABELS, Usage, compact, levels, row_of};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use time::{Date, Duration, Month};

/// Width of the `Mon ` column.
const LABEL: usize = 4;
/// Columns per week: the cell and a space.
const CELL: usize = 2;
/// Two years: as far back as the server keeps days.
const MAX_WEEKS: usize = 105;

const SHADES: [Color; 5] = [
    Color::Rgb(0x24, 0x24, 0x24),
    Color::Rgb(0x3b, 0x22, 0x63),
    Color::Rgb(0x5b, 0x2b, 0xa8),
    Color::Rgb(0x84, 0x42, 0xd9),
    Color::Rgb(0xc4, 0xa1, 0xff),
];

const BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Weeks that fit in `width` columns.
pub fn weeks_for(width: usize) -> usize {
    (width.saturating_sub(LABEL) / CELL).clamp(1, MAX_WEEKS)
}

/// A month line over seven day rows, Monday first. Days after today stay
/// empty.
pub fn grid(usage: &Usage, width: usize) -> Vec<Line<'static>> {
    let weeks = weeks_for(width);
    let start = usage.chart_start(weeks);
    let per_day = usage.per_day();
    let days: Vec<Date> = (0..weeks * 7)
        .map(|offset| start + Duration::days(i64::try_from(offset).unwrap_or(0)))
        .collect();
    let values: Vec<u64> = days
        .iter()
        .map(|day| per_day.get(day).copied().unwrap_or(0))
        .collect();
    let shade = levels(&values);

    let mut lines = vec![month_line(start, weeks)];
    for row in 0..7 {
        let label = ROW_LABELS
            .iter()
            .find(|(weekday, _)| usize::from(weekday.number_days_from_monday()) == row)
            .map_or("", |(_, label)| label);
        let mut spans = vec![Span::styled(format!("{label:<LABEL$}"), theme::meta())];
        for week in 0..weeks {
            let index = week * 7 + row;
            let day = days[index];
            debug_assert_eq!(row_of(day), row);
            if day > usage.today {
                spans.push(Span::raw("  "));
            } else {
                let colour = SHADES[usize::from(shade[index])];
                spans.push(Span::styled("■ ", Style::new().fg(colour)));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Month names over the weeks they begin in.
fn month_line(start: Date, weeks: usize) -> Line<'static> {
    let starts: Vec<(usize, Date)> = (0..weeks)
        .filter_map(|week| {
            let monday = start + Duration::days(i64::try_from(week * 7).unwrap_or(0));
            (0..7)
                .map(|offset| monday + Duration::days(offset))
                .find(|day| day.day() == 1)
                .map(|first| (LABEL + week * CELL, first))
        })
        .collect();
    let mut text = " ".repeat(LABEL + weeks * CELL);
    let mut free_from = 0;
    for &(at, first) in &starts {
        let name = short_month(first.month());
        if at >= free_from && at + name.len() <= text.len() {
            text.replace_range(at..at + name.len(), name);
            free_from = at + name.len() + 1;
        }
    }
    Line::styled(text.trim_end().to_string(), theme::meta())
}

/// `Less ■ ■ ■ ■ ■ More`
pub fn legend() -> Line<'static> {
    let mut spans = vec![Span::styled("Less ", theme::meta())];
    for colour in SHADES {
        spans.push(Span::styled("■ ", Style::new().fg(colour)));
    }
    spans.push(Span::styled("More", theme::meta()));
    Line::from(spans)
}

/// Width of the y-axis labels left of the columns (`796M ┤`).
const AXIS: usize = 8;
/// The chart never goes back less than this, so a new account isn't one fat bar.
const MIN_DAYS: usize = 14;

/// The days the column chart shows: from the first day with tokens (at
/// least two weeks back) to today, no more than `width` of them.
pub fn chart_days(usage: &Usage, width: usize) -> usize {
    let most = width.saturating_sub(AXIS).max(MIN_DAYS);
    let first = usage.per_day().into_iter().find(|(_, total)| *total > 0);
    let since = first.map_or(MIN_DAYS, |(day, _)| {
        usize::try_from((usage.today - day).whole_days() + 1).unwrap_or(MIN_DAYS)
    });
    since.clamp(MIN_DAYS, most)
}

/// Where each day's slot starts across `width` columns: the days share the
/// whole width, so the chart always spans it.
fn slots(days: usize, width: usize) -> Vec<(usize, usize)> {
    (0..days)
        .map(|day| (day * width / days, (day + 1) * width / days))
        .collect()
}

/// Tokens per day as columns across the whole `width`, `height` rows tall:
/// a y-axis with the peak and half of it, a baseline, and month names
/// under the days they start.
pub fn columns(usage: &Usage, days: usize, width: usize, height: usize) -> Vec<Line<'static>> {
    let per_day = usage.per_day();
    let days = days.max(1);
    let first = usage.today - Duration::days(i64::try_from(days - 1).unwrap_or(0));
    let values: Vec<u64> = (0..days)
        .map(|offset| {
            let day = first + Duration::days(i64::try_from(offset).unwrap_or(0));
            per_day.get(&day).copied().unwrap_or(0)
        })
        .collect();
    let peak = values.iter().copied().max().unwrap_or(0);
    let plot = width.saturating_sub(AXIS).max(days);
    let slots = slots(days, plot);
    let eighths: Vec<usize> = values
        .iter()
        .map(|&value| {
            if peak == 0 || value == 0 {
                0
            } else {
                // Any usage shows at least a sliver.
                ((value as f64 / peak as f64 * (height * 8) as f64).round() as usize).max(1)
            }
        })
        .collect();

    let axis = |row: usize| -> String {
        let label = if peak == 0 {
            String::new()
        } else if row == 0 {
            compact(peak)
        } else if row == height / 2 && height >= 4 {
            compact(peak / 2)
        } else {
            String::new()
        };
        let tick = if label.is_empty() { "│" } else { "┤" };
        format!("{label:>width$} {tick}", width = AXIS - 2)
    };
    let mut lines = Vec::new();
    for row in 0..height {
        let floor = (height - 1 - row) * 8;
        let mut spans = vec![Span::styled(axis(row), theme::meta())];
        for (index, (&(from, to), &fill)) in slots.iter().zip(&eighths).enumerate() {
            let slot = to - from;
            // A column of space between days once there's room for it.
            let bar = if slot >= 3 { slot - 1 } else { slot };
            let part = fill.saturating_sub(floor).min(8);
            let block = if part == 0 { " " } else { BLOCKS[part - 1] };
            let style = if index + 1 == days {
                theme::accent()
            } else {
                theme::brand()
            };
            spans.push(Span::styled(block.repeat(bar), style));
            spans.push(Span::raw(" ".repeat(slot - bar)));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(vec![
        Span::styled(
            format!("{:>width$} └", "0", width = AXIS - 2),
            theme::meta(),
        ),
        Span::styled("─".repeat(plot), Style::default().fg(theme::BORDER)),
    ]));
    lines.push(day_labels(first, &slots));
    lines
}

/// The first day, each month's first day and "Today", under their columns;
/// a label that would run into the one before it is left out.
fn day_labels(first: Date, slots: &[(usize, usize)]) -> Line<'static> {
    let last = slots.len().saturating_sub(1);
    let plot = slots.last().map_or(0, |slot| slot.1);
    let mut marks: Vec<(usize, String)> = Vec::new();
    for (index, &(from, _)) in slots.iter().enumerate() {
        let day = first + Duration::days(i64::try_from(index).unwrap_or(0));
        let label = if index == 0 || day.day() == 1 {
            format!("{} {}", short_month(day.month()), day.day())
        } else {
            continue;
        };
        marks.push((from, label));
    }
    let today = "Today";
    let mut text = String::new();
    for (at, label) in marks {
        let room = plot.saturating_sub(today.len() + 1);
        if at < text.chars().count() + usize::from(!text.is_empty()) || at + label.len() > room {
            continue;
        }
        text.push_str(&" ".repeat(at - text.chars().count()));
        text.push_str(&label);
    }
    let today_at = slots.get(last).map_or(0, |&(from, to)| {
        (from + to)
            .saturating_sub(today.len())
            .min(plot.saturating_sub(today.len()))
    });
    let today_at = today_at.max(text.chars().count() + 1);
    text.push_str(&" ".repeat(today_at - text.chars().count()));
    text.push_str(today);
    Line::styled(format!("{}{text}", " ".repeat(AXIS)), theme::meta())
}

/// `name  ━━━━━━━━  1.2M`, the bar scaled to `max`.
pub fn bar_line(
    name: &str,
    value: u64,
    max: u64,
    name_width: usize,
    bar_width: usize,
) -> Line<'static> {
    let filled = if max == 0 || value == 0 {
        0
    } else {
        ((value as f64 / max as f64 * bar_width as f64).round() as usize).clamp(1, bar_width)
    };
    let name: String = if name.chars().count() > name_width {
        let mut cut: String = name.chars().take(name_width.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        name.to_string()
    };
    Line::from(vec![
        Span::styled(format!("{name:<name_width$}  "), theme::text()),
        Span::styled("━".repeat(filled), theme::brand()),
        Span::raw(" ".repeat(bar_width - filled + 2)),
        Span::styled(compact(value), theme::muted()),
    ])
}

pub fn short_month(month: Month) -> &'static str {
    match month {
        Month::January => "Jan",
        Month::February => "Feb",
        Month::March => "Mar",
        Month::April => "Apr",
        Month::May => "May",
        Month::June => "Jun",
        Month::July => "Jul",
        Month::August => "Aug",
        Month::September => "Sep",
        Month::October => "Oct",
        Month::November => "Nov",
        Month::December => "Dec",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::tests::usage;

    fn text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_grid_has_a_month_line_and_seven_days() {
        let lines = grid(&usage(), 4 + 2 * 3);
        assert_eq!(lines.len(), 8);
        // Sep 21 – Oct 11: October starts in the third week.
        assert_eq!(text(&lines[0]), "      Oct");
        assert!(text(&lines[1]).starts_with("Mon ■ ■ ■"));
        // Today is Friday: Saturday and Sunday of this week stay empty.
        assert_eq!(text(&lines[6]), "    ■ ■   ");
        assert!(text(&lines[5]).starts_with("Fri "));
    }

    #[test]
    fn wide_terminals_go_back_further() {
        let usage = usage();
        assert_eq!(weeks_for(4 + 2 * 80), 80);
        assert_eq!(weeks_for(1000), MAX_WEEKS);
        let months = text(&grid(&usage, 4 + 2 * 60)[0]);
        // 60 weeks back from Oct 2026 starts mid-August 2025.
        assert!(months.trim_start().starts_with("Sep "), "{months}");
        assert!(months.contains(" Jan "), "{months}");
        assert!(!months.contains('\''), "{months}");
        assert!(months.trim_end().ends_with("Oct"), "{months}");
    }

    #[test]
    fn busier_days_are_darker() {
        let lines = grid(&usage(), 4 + 2 * 3);
        // Wed Sep 30 had the most tokens; Thu Oct 1 the next most.
        let colour = |line: usize, week: usize| lines[line].spans[1 + week].style.fg;
        assert_eq!(colour(3, 1), Some(SHADES[4]));
        assert_eq!(colour(1, 0), Some(SHADES[0]));
    }

    #[test]
    fn the_chart_starts_at_the_first_day_with_tokens() {
        // Sep 30 is the first: 10 days, but never less than two weeks.
        assert_eq!(chart_days(&usage(), 200), MIN_DAYS);
        let mut long = usage();
        long.days[4].day = time::macros::date!(2026 - 07 - 01);
        assert_eq!(chart_days(&long, 200), 101);
        // A narrow terminal shows what fits.
        assert_eq!(chart_days(&long, AXIS + 40), 40);
    }

    #[test]
    fn columns_fill_the_whole_width() {
        let width = AXIS + 70;
        let lines = columns(&usage(), 14, width, 4);
        // Rows, baseline, labels.
        assert_eq!(lines.len(), 6);
        for line in &lines[..5] {
            assert_eq!(text(line).chars().count(), width, "{}", text(line));
        }
        // The peak (Sep 30, 8K) is labelled and reaches the top row.
        assert!(text(&lines[0]).starts_with("  8.0K ┤"));
        assert!(text(&lines[2]).starts_with("  4.0K ┤"));
        assert!(text(&lines[4]).starts_with("     0 └───"));
        let labels = text(&lines[5]);
        assert!(
            labels.contains("Sep 26") && labels.contains("Oct 1"),
            "{labels}"
        );
        assert!(labels.ends_with("Today"), "{labels}");
        // Sep 30 is day 5 of 14: its slot starts at 4 * 70 / 14 = 20.
        let top = text(&lines[0]);
        assert_eq!(top.chars().nth(AXIS + 20), Some('█'));
        assert_eq!(top.chars().nth(AXIS + 19), Some(' '));
    }

    #[test]
    fn a_quiet_range_draws_only_the_axis() {
        let mut quiet = usage();
        quiet.days.clear();
        let lines = columns(&quiet, 14, 40, 3);
        assert!(text(&lines[0]).trim().starts_with('│'));
        assert!(text(&lines[4]).ends_with("Today"));
    }

    #[test]
    fn bar_lines_shorten_long_names() {
        let line = bar_line("claude-opus-4-1-20250805", 50, 100, 10, 8);
        assert_eq!(text(&line), "claude-op…  ━━━━      50");
    }
}
