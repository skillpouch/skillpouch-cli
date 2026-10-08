//! Small pieces every screen uses.

use crate::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(tick: usize) -> Span<'static> {
    Span::styled(SPINNER[tick % SPINNER.len()], theme::accent())
}

/// A section title inside a page.
pub fn heading(text: &str) -> Line<'_> {
    Line::styled(text, theme::strong())
}

/// `label    value`, the label in a fixed column.
pub fn field<'a>(name: &str, value: impl Into<Vec<Span<'a>>>) -> Line<'a> {
    let mut spans = vec![Span::styled(format!("{name:<14}"), theme::meta())];
    spans.extend(value.into());
    Line::from(spans)
}

/// A field with one value per line, lined up under the first.
pub fn list_field<'a>(name: &str, values: &'a [impl AsRef<str>]) -> Vec<Line<'a>> {
    if values.is_empty() {
        return vec![field(name, vec![Span::styled("None found", theme::meta())])];
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let label = if index == 0 { name } else { "" };
            field(label, vec![Span::styled(value.as_ref(), theme::text())])
        })
        .collect()
}

/// `key label   key label`, for the footer and the login screen.
pub fn keys<'a>(pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = Vec::new();
    for (index, (key, what)) in pairs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("    "));
        }
        spans.push(Span::styled(*key, theme::text()));
        spans.push(Span::styled(format!(" {what}"), theme::meta()));
    }
    Line::from(spans)
}

/// `━━━━━━────`: purple, yellow from 75 %, red from 90 %.
pub fn meter(used: u64, total: u64, width: usize) -> Vec<Span<'static>> {
    let ratio = if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    };
    let mut filled = ((ratio * width as f64).round() as usize).min(width);
    if used > 0 && filled == 0 && width > 0 {
        filled = 1;
    }
    let colour = if ratio >= 0.9 {
        theme::error()
    } else if ratio >= 0.75 {
        theme::warn()
    } else {
        theme::brand()
    };
    vec![
        Span::styled("━".repeat(filled), colour),
        Span::styled("━".repeat(width - filled), Style::new().fg(theme::BORDER)),
    ]
}

/// `1536` → `1.5 KB`.
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

pub fn state_style(state: &str) -> Style {
    match state {
        "active" => theme::muted(),
        "locked" | "suspended" => theme::warn(),
        "deleted" | "revoked" => theme::error(),
        _ => theme::meta(),
    }
}

pub fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

pub fn key_storage(kind: &str) -> &str {
    match kind {
        "tpm" => "TPM",
        "secure_enclave" => "Secure Enclave",
        "os_keystore" => "System keychain",
        _ => "Encrypted file",
    }
}

/// A `width` × `height` box in the middle of `area`.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn meters_fill_and_change_colour() {
        assert_eq!(meter(0, 0, 4)[0].content, "");
        assert_eq!(meter(50, 100, 4)[0].content, "━━");
        // Anything stored shows at least one segment.
        assert_eq!(meter(1, 1_000_000, 4)[0].content, "━");
        assert_eq!(meter(95, 100, 4)[0].style, theme::error());
    }
}
