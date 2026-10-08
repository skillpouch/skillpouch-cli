//! The CLI's colours, for ratatui.

use crate::logo::{BRAND, Rgb};
use ratatui::style::{Color, Style};

const ACCENT_LIGHT: Rgb = Rgb(0xb3, 0x8b, 0xf0);
const BORDER: Rgb = Rgb(0x3a, 0x3a, 0x3a);

pub fn rgb(colour: Rgb) -> Color {
    Color::Rgb(colour.0, colour.1, colour.2)
}

/// The logo's purple.
pub fn brand() -> Style {
    Style::new().fg(rgb(BRAND))
}

/// Lighter purple, readable as text on black.
pub fn accent() -> Style {
    Style::new().fg(rgb(ACCENT_LIGHT))
}

pub fn border() -> Style {
    Style::new().fg(rgb(BORDER))
}

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn warn() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn error() -> Style {
    Style::new().fg(Color::Red)
}
