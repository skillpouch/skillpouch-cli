//! A titled box of label/value rows, and a usage meter to put in one.
//!
//! ```text
//! ╭─ Account ───────────────────────────╮
//! │  Plan       Supporter+               │
//! │  Storage    ██████░░░░░░  1.2 GB of 10 GB │
//! ╰──────────────────────────────────────╯
//! ```

use super::theme::{dim, heading, shades};
use console::{measure_text_width, truncate_str};

const MIN_WIDTH: usize = 44;
const PADDING: usize = 2;
const GAP: usize = 3;

pub struct Card {
    title: String,
    rows: Vec<(String, String)>,
}

impl Card {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
        }
    }

    pub fn row(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.rows.push((label.into(), value.into()));
        self
    }

    /// The card at most `max_width` columns wide; longer values are cut.
    pub fn render(&self, max_width: usize) -> String {
        let label_width = self
            .rows
            .iter()
            .map(|(label, _)| measure_text_width(label))
            .max()
            .unwrap_or(0);
        let content = self
            .rows
            .iter()
            .map(|(_, value)| label_width + GAP + measure_text_width(value))
            .max()
            .unwrap_or(0)
            .max(measure_text_width(&self.title) + 2);
        let inner = (content + 2 * PADDING)
            .max(MIN_WIDTH)
            .min(max_width.saturating_sub(2).max(MIN_WIDTH));
        let room = inner.saturating_sub(2 * PADDING + label_width + GAP);
        let border = &shades(false)[1];

        let title = format!(" {} ", heading(&self.title));
        let rule = inner.saturating_sub(1 + measure_text_width(&title));
        let mut out = format!(
            "{}{}{}\n",
            border.apply_to("╭─"),
            title,
            border.apply_to(format!("{}╮", "─".repeat(rule)))
        );
        for (label, value) in &self.rows {
            let value = truncate_str(value, room, "…");
            let used = PADDING + label_width + GAP + measure_text_width(&value);
            out.push_str(&format!(
                "{}{}{}{}{}{}{}\n",
                border.apply_to("│"),
                " ".repeat(PADDING),
                dim(format!("{label:<label_width$}")),
                " ".repeat(GAP),
                value,
                " ".repeat(inner.saturating_sub(used)),
                border.apply_to("│"),
            ));
        }
        out.push_str(
            &border
                .apply_to(format!("╰{}╯", "─".repeat(inner)))
                .to_string(),
        );
        out
    }
}

/// `████░░░░` filled to `used / total`, `width` cells wide.
pub fn meter(used: u64, total: u64, width: usize) -> String {
    let ratio = if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    };
    let mut filled = (ratio * width as f64).round() as usize;
    if used > 0 && total > 0 && filled == 0 {
        filled = 1;
    }
    let color = if ratio >= 0.9 {
        console::Style::new().red()
    } else if ratio >= 0.75 {
        console::Style::new().yellow()
    } else {
        shades(false)[0].clone()
    };
    format!(
        "{}{}",
        color.apply_to("█".repeat(filled)),
        dim("░".repeat(width - filled))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() {
        console::set_colors_enabled(false);
    }

    #[test]
    fn rows_line_up_inside_the_border() {
        plain();
        let card = Card::new("Account")
            .row("Plan", "Free")
            .row("Server", "https://skillpouch.net")
            .render(80);
        let lines: Vec<&str> = card.lines().collect();
        assert_eq!(lines.len(), 4);
        let width = measure_text_width(lines[0]);
        assert!(lines.iter().all(|line| measure_text_width(line) == width));
        assert!(lines[0].starts_with("╭─ Account "));
        assert!(lines[1].starts_with("│  Plan     Free"));
    }

    #[test]
    fn long_values_are_cut_to_the_terminal() {
        plain();
        let card = Card::new("x").row("Server", "y".repeat(200)).render(60);
        assert!(card.lines().all(|line| measure_text_width(line) <= 60));
        assert!(card.contains('…'));
    }

    #[test]
    fn meters_fill_in_proportion() {
        plain();
        assert_eq!(meter(0, 100, 10), "░".repeat(10));
        assert_eq!(
            meter(50, 100, 10),
            format!("{}{}", "█".repeat(5), "░".repeat(5))
        );
        assert_eq!(meter(1, 1_000_000, 4), format!("█{}", "░".repeat(3)));
        assert_eq!(meter(9, 0, 4), "░".repeat(4));
    }
}
