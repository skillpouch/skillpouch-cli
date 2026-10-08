//! Server timestamps as people read them: "3 min ago", "Oct 8".

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn parse(timestamp: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(timestamp, &Rfc3339).ok()
}

/// `2026-10-08T…` → `Oct 8`, with the year when it isn't this one.
pub fn date(timestamp: &str, now: OffsetDateTime) -> Option<String> {
    let at = parse(timestamp)?;
    let month = MONTHS[usize::from(u8::from(at.month())) - 1];
    Some(if at.year() == now.year() {
        format!("{month} {}", at.day())
    } else {
        format!("{month} {}, {}", at.day(), at.year())
    })
}

/// How long ago: `just now`, `5 min ago`, `3 h ago`, `2 days ago`, then the date.
pub fn ago(timestamp: &str, now: OffsetDateTime) -> Option<String> {
    let seconds = (now - parse(timestamp)?).whole_seconds().max(0);
    Some(match seconds {
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{} h ago", seconds / 3_600),
        86_400..172_800 => "yesterday".into(),
        172_800..604_800 => format!("{} days ago", seconds / 86_400),
        _ => return date(timestamp, now),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> OffsetDateTime {
        OffsetDateTime::parse("2026-10-09T12:00:00Z", &Rfc3339)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }

    #[test]
    fn reads_like_a_person_would_say_it() {
        let cases = [
            ("2026-10-09T11:59:30Z", "just now"),
            ("2026-10-09T11:54:59.500Z", "5 min ago"),
            ("2026-10-09T09:00:00+00:00", "3 h ago"),
            ("2026-10-08T10:00:00Z", "yesterday"),
            ("2026-10-06T12:00:00Z", "3 days ago"),
            ("2026-09-01T12:00:00Z", "Sep 1"),
            ("2025-12-24T12:00:00Z", "Dec 24, 2025"),
        ];
        for (timestamp, expected) in cases {
            assert_eq!(
                ago(timestamp, now()).as_deref(),
                Some(expected),
                "{timestamp}"
            );
        }
        assert_eq!(ago("yesterday", now()), None);
    }
}
