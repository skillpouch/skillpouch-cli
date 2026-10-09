//! Token usage summed over every computer: totals per period, one number
//! per day for the chart, and breakdowns by model and assistant.

use std::collections::BTreeMap;
use time::{Date, Duration, Weekday};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    fn add(&mut self, other: &Tokens) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
    }
}

/// One UTC day of one model in one assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDay {
    pub day: Date,
    pub assistant: String,
    pub model: String,
    pub tokens: Tokens,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    /// Today in UTC, the day the server counts in.
    pub today: Date,
    pub days: Vec<UsageDay>,
    pub all_time: Tokens,
}

/// The periods the totals are shown for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Today,
    Week,
    Month,
}

impl Period {
    pub const ALL: [Period; 3] = [Self::Today, Self::Week, Self::Month];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Week => "This week",
            Self::Month => "This month",
        }
    }
}

impl Usage {
    /// The first day of `period`; weeks start on Monday.
    pub fn start(&self, period: Period) -> Date {
        match period {
            Period::Today => self.today,
            Period::Week => {
                let back = self.today.weekday().number_days_from_monday();
                self.today - Duration::days(i64::from(back))
            }
            Period::Month => self.today.replace_day(1).unwrap_or(self.today),
        }
    }

    pub fn since(&self, from: Date) -> Tokens {
        let mut sum = Tokens::default();
        for day in self.days.iter().filter(|d| d.day >= from) {
            sum.add(&day.tokens);
        }
        sum
    }

    pub fn total(&self, period: Period) -> Tokens {
        self.since(self.start(period))
    }

    /// All tokens per day.
    pub fn per_day(&self) -> BTreeMap<Date, u64> {
        let mut days = BTreeMap::new();
        for day in &self.days {
            *days.entry(day.day).or_insert(0u64) += day.tokens.total();
        }
        days
    }

    /// Tokens since `from` per model, largest first.
    pub fn by_model(&self, from: Date) -> Vec<(String, u64)> {
        self.group(from, |day| day.model.as_str())
    }

    /// Tokens since `from` per assistant id, largest first.
    pub fn by_assistant(&self, from: Date) -> Vec<(String, u64)> {
        self.group(from, |day| day.assistant.as_str())
    }

    fn group(&self, from: Date, key: impl Fn(&UsageDay) -> &str) -> Vec<(String, u64)> {
        let mut groups: BTreeMap<&str, u64> = BTreeMap::new();
        for day in self.days.iter().filter(|d| d.day >= from) {
            *groups.entry(key(day)).or_insert(0) += day.tokens.total();
        }
        let mut sorted: Vec<(String, u64)> = groups
            .into_iter()
            .filter(|(_, total)| *total > 0)
            .map(|(key, total)| (key.to_string(), total))
            .collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        sorted
    }

    /// The Monday that starts the chart when it shows `weeks` weeks ending
    /// with this one.
    pub fn chart_start(&self, weeks: usize) -> Date {
        let this_week = self.start(Period::Week);
        let back = i64::try_from(weeks.saturating_sub(1)).unwrap_or(0) * 7;
        this_week - Duration::days(back)
    }
}

/// How dark a chart cell is: 0 for nothing, then 1–4 by rank among the busy
/// days shown, so a few huge days don't wash out the rest and the busiest
/// day is always the darkest.
pub fn levels(values: &[u64]) -> Vec<u8> {
    let mut busy: Vec<u64> = values.iter().copied().filter(|v| *v > 0).collect();
    busy.sort_unstable();
    values
        .iter()
        .map(|&value| {
            if value == 0 {
                return 0;
            }
            let rank = busy.partition_point(|other| *other <= value);
            u8::try_from((rank * 4).div_ceil(busy.len())).unwrap_or(4)
        })
        .collect()
}

/// `1234567` → `1.2M`.
pub fn compact(value: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1_000_000_000_000, "T"),
        (1_000_000_000, "B"),
        (1_000_000, "M"),
        (1_000, "K"),
    ];
    for (size, unit) in UNITS {
        if value >= size {
            let scaled = value as f64 / size as f64;
            return if scaled >= 100.0 {
                format!("{scaled:.0}{unit}")
            } else {
                format!("{scaled:.1}{unit}")
            };
        }
    }
    value.to_string()
}

/// Monday-first row of a day in the chart.
pub fn row_of(day: Date) -> usize {
    usize::from(day.weekday().number_days_from_monday())
}

pub const ROW_LABELS: [(Weekday, &str); 3] = [
    (Weekday::Monday, "Mon"),
    (Weekday::Wednesday, "Wed"),
    (Weekday::Friday, "Fri"),
];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use time::macros::date;

    fn day(day: Date, assistant: &str, model: &str, total: u64) -> UsageDay {
        UsageDay {
            day,
            assistant: assistant.into(),
            model: model.into(),
            tokens: Tokens {
                input: total / 2,
                output: total - total / 2,
                ..Tokens::default()
            },
        }
    }

    pub fn usage() -> Usage {
        Usage {
            // A Friday.
            today: date!(2026 - 10 - 09),
            days: vec![
                day(date!(2026 - 10 - 09), "claude-code", "claude-opus", 1_000),
                day(date!(2026 - 10 - 09), "codex", "gpt-5", 500),
                day(date!(2026 - 10 - 05), "claude-code", "claude-opus", 2_000),
                day(date!(2026 - 10 - 01), "opencode", "claude-sonnet", 4_000),
                day(date!(2026 - 09 - 30), "claude-code", "claude-opus", 8_000),
            ],
            all_time: Tokens {
                input: 10_000,
                output: 5_500,
                ..Tokens::default()
            },
        }
    }

    #[test]
    fn totals_count_calendar_periods_in_utc() {
        let usage = usage();
        assert_eq!(usage.start(Period::Week), date!(2026 - 10 - 05));
        assert_eq!(usage.total(Period::Today).total(), 1_500);
        assert_eq!(usage.total(Period::Week).total(), 3_500);
        assert_eq!(usage.total(Period::Month).total(), 7_500);
    }

    #[test]
    fn breakdowns_are_largest_first() {
        let usage = usage();
        let from = usage.start(Period::Month);
        assert_eq!(
            usage.by_model(from),
            [
                ("claude-sonnet".to_string(), 4_000),
                ("claude-opus".to_string(), 3_000),
                ("gpt-5".to_string(), 500),
            ]
        );
        assert_eq!(usage.by_assistant(from)[0], ("opencode".to_string(), 4_000));
        assert_eq!(usage.per_day()[&date!(2026 - 10 - 09)], 1_500);
    }

    #[test]
    fn the_chart_starts_on_a_monday() {
        let usage = usage();
        assert_eq!(usage.chart_start(1), date!(2026 - 10 - 05));
        assert_eq!(usage.chart_start(3), date!(2026 - 09 - 21));
        assert_eq!(row_of(date!(2026 - 10 - 05)), 0);
        assert_eq!(row_of(date!(2026 - 10 - 11)), 6);
    }

    #[test]
    fn levels_follow_the_rank_of_busy_days() {
        assert_eq!(levels(&[0, 1, 2, 3, 4, 1_000_000]), [0, 1, 2, 3, 4, 4]);
        assert_eq!(levels(&[0, 0]), [0, 0]);
        // One busy day, or several equal ones, are the darkest.
        assert_eq!(levels(&[5]), [4]);
        assert_eq!(levels(&[0, 7, 7]), [0, 4, 4]);
    }

    #[test]
    fn numbers_are_compact() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_234), "1.2K");
        assert_eq!(compact(12_345_678), "12.3M");
        assert_eq!(compact(250_000_000), "250M");
        assert_eq!(compact(3_100_000_000), "3.1B");
    }
}
