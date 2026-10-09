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

/// The last `days` days as bars `height` rows tall, with the first date and
/// "Today" under them.
pub fn bars(usage: &Usage, days: usize, height: usize) -> Vec<Line<'static>> {
    let per_day = usage.per_day();
    let first = usage.today - Duration::days(i64::try_from(days.saturating_sub(1)).unwrap_or(0));
    let values: Vec<u64> = (0..days)
        .map(|offset| {
            let day = first + Duration::days(i64::try_from(offset).unwrap_or(0));
            per_day.get(&day).copied().unwrap_or(0)
        })
        .collect();
    let peak = values.iter().copied().max().unwrap_or(0);
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

    let mut lines = Vec::new();
    for row in 0..height {
        let floor = (height - 1 - row) * 8;
        let spans: Vec<Span<'static>> = eighths
            .iter()
            .enumerate()
            .map(|(index, &fill)| {
                let part = fill.saturating_sub(floor).min(8);
                let block = if part == 0 { " " } else { BLOCKS[part - 1] };
                let style = if index + 1 == days {
                    theme::accent()
                } else {
                    theme::brand()
                };
                Span::styled(format!("{block} "), style)
            })
            .collect();
        lines.push(Line::from(spans));
    }
    let start = format!("{} {}", short_month(first.month()), first.day());
    let width = days * CELL - 1;
    let gap = width.saturating_sub(start.len() + "Today".len());
    lines.push(Line::styled(
        format!("{start}{}Today", " ".repeat(gap)),
        theme::meta(),
    ));
    lines
}

/// `name  ━━━━━━━━  1.2M`, the bar scaled to `max`.
pub fn bar_line(
    name: &str,
    value: u64,
    max: u64,
    name_width: usize,
    bar_width: usize,
) -> Line<'static> {
    let filled = if max == 0 {
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
    fn bars_scale_to_the_peak_and_label_the_ends() {
        let lines = bars(&usage(), 10, 2);
        assert_eq!(lines.len(), 3);
        // Sep 30 (8K, the peak) fills both rows; Oct 1 (4K) only the lower.
        assert!(text(&lines[0]).starts_with("█   "));
        assert!(text(&lines[1]).starts_with("█ █ "));
        assert_eq!(text(&lines[2]), format!("Sep 30{}Today", " ".repeat(8)));
    }

    #[test]
    fn bar_lines_shorten_long_names() {
        let line = bar_line("claude-opus-4-1-20250805", 50, 100, 10, 8);
        assert_eq!(text(&line), "claude-op…  ━━━━      50");
    }
}
