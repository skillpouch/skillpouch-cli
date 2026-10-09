//! Token usage: read from the assistants' own logs on this computer, sent to
//! the server per day and model, and read back summed over every computer.
//!
//! Each computer reports under its own source id and the server keeps one
//! row per source, day, assistant and model, so sending a day again
//! replaces it instead of adding to it.

use crate::context::{Context, Session};
use sp_api::types::{UsageCounts, UsageReport, UsageRow, UsageSummary};
use sp_core::assistants;
use sp_core::home::user_home;
use sp_tui::usage::{Tokens, Usage, UsageDay};
use sp_usage::{DayUsage, Locations};
use std::collections::HashMap;
use std::sync::Mutex;
use time::{Date, Month, OffsetDateTime};

/// Most rows the server takes per report.
const REPORT_ROWS: usize = 2000;

type RowKey = (String, String, String);

/// What this computer already sent, so a refresh only sends what changed.
#[derive(Default)]
pub struct Reported(Mutex<HashMap<RowKey, UsageCounts>>);

impl Reported {
    /// Rows that are new or changed since the last successful report.
    fn changed(&self, rows: Vec<UsageRow>) -> Vec<UsageRow> {
        let sent = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        rows.into_iter()
            .filter(|row| sent.get(&key(row)) != Some(&row.counts))
            .collect()
    }

    fn remember(&self, rows: &[UsageRow]) {
        let mut sent = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for row in rows {
            sent.insert(key(row), row.counts);
        }
    }
}

fn key(row: &UsageRow) -> RowKey {
    (row.day.clone(), row.assistant.clone(), row.model.clone())
}

/// Sends this computer's new usage and returns the account's.
pub async fn sync(ctx: &Context, session: &Session, reported: &Reported) -> Result<Usage, String> {
    let scanned = tokio::task::spawn_blocking(scan)
        .await
        .map_err(|error| format!("Couldn't read token usage: {error}."))?;
    if let Some((source, rows)) = scanned {
        let rows = reported.changed(rows);
        let token = session.credentials.access_token.as_str();
        for chunk in rows.chunks(REPORT_ROWS) {
            let report = UsageReport {
                rows: chunk.to_vec(),
            };
            ctx.client
                .report_usage(&session.key, token, &source, &report)
                .await
                .map_err(|error| format!("Couldn't send this computer's token usage: {error}."))?;
            reported.remember(chunk);
        }
    }
    let summary = ctx
        .client
        .usage(&session.key, &session.credentials.access_token)
        .await
        .map_err(|error| format!("Couldn't load token usage: {error}."))?;
    Ok(usage(summary, OffsetDateTime::now_utc().date()))
}

/// This computer's source id and its usage rows.
fn scan() -> Option<(String, Vec<UsageRow>)> {
    let home = user_home()?;
    let rows = sp_usage::scan(&Locations::of(&home))
        .into_iter()
        .map(row)
        .collect();
    Some((sp_usage::source_id(&home), rows))
}

fn row(day: DayUsage) -> UsageRow {
    UsageRow {
        day: day.day.to_string(),
        assistant: day.assistant,
        model: day.model,
        counts: UsageCounts {
            input: day.counts.input,
            output: day.counts.output,
            cache_read: day.counts.cache_read,
            cache_write: day.counts.cache_write,
        },
    }
}

/// The server's sums, for the TUI. Assistants show by name.
fn usage(summary: UsageSummary, today: Date) -> Usage {
    let days = summary
        .days
        .into_iter()
        .filter_map(|row| {
            Some(UsageDay {
                day: parse_day(&row.day)?,
                assistant: assistants::name_of(&row.assistant)
                    .map_or(row.assistant, str::to_string),
                model: row.model,
                tokens: tokens(row.counts),
            })
        })
        .collect();
    Usage {
        today,
        days,
        all_time: tokens(summary.all_time),
    }
}

fn tokens(counts: UsageCounts) -> Tokens {
    Tokens {
        input: counts.input,
        output: counts.output,
        cache_read: counts.cache_read,
        cache_write: counts.cache_write,
    }
}

/// `2026-10-09`
fn parse_day(text: &str) -> Option<Date> {
    let mut parts = text.splitn(3, '-');
    let year = parts.next()?.parse().ok()?;
    let month = Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.parse().ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_row(day: &str, model: &str, input: u64) -> UsageRow {
        UsageRow {
            day: day.into(),
            assistant: "claude-code".into(),
            model: model.into(),
            counts: UsageCounts {
                input,
                ..UsageCounts::default()
            },
        }
    }

    #[test]
    fn only_new_or_changed_rows_are_sent_again() {
        let reported = Reported::default();
        let first = vec![
            usage_row("2026-10-08", "opus", 10),
            usage_row("2026-10-09", "opus", 5),
        ];
        assert_eq!(reported.changed(first.clone()).len(), 2);
        reported.remember(&first);
        let later = vec![
            usage_row("2026-10-08", "opus", 10),
            usage_row("2026-10-09", "opus", 9),
            usage_row("2026-10-09", "sonnet", 1),
        ];
        let changed = reported.changed(later);
        assert_eq!(
            changed,
            [
                usage_row("2026-10-09", "opus", 9),
                usage_row("2026-10-09", "sonnet", 1)
            ]
        );
    }

    #[test]
    fn days_print_and_parse_as_the_server_writes_them() {
        let day = Date::from_calendar_date(2026, Month::October, 9).ok();
        assert_eq!(day.map(|d| d.to_string()).as_deref(), Some("2026-10-09"));
        assert_eq!(parse_day("2026-10-09"), day);
        assert_eq!(parse_day("2026-13-01"), None);
        assert_eq!(parse_day("nope"), None);
    }

    #[test]
    fn the_summary_names_assistants() {
        let summary = UsageSummary {
            days: vec![
                usage_row("2026-10-09", "opus", 7),
                UsageRow {
                    assistant: "mystery".into(),
                    ..usage_row("2026-10-09", "x", 1)
                },
            ],
            all_time: UsageCounts {
                input: 8,
                ..UsageCounts::default()
            },
            updated_at: None,
        };
        let today = Date::from_calendar_date(2026, Month::October, 9).unwrap_or(Date::MIN);
        let usage = usage(summary, today);
        assert_eq!(usage.days[0].assistant, "Claude Code");
        assert_eq!(usage.days[1].assistant, "mystery");
        assert_eq!(usage.all_time.total(), 8);
    }
}
