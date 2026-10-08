//! The CLI's colours, for ratatui: black background, brand purple accents.

use crate::logo::BRAND;
use ratatui::style::{Color, Modifier, Style};

const fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

pub const PURPLE: Color = rgb(BRAND);
pub const LIGHT: Color = rgb((0xb3, 0x8b, 0xf0));
pub const BORDER: Color = rgb((0x2e, 0x2e, 0x2e));
pub const BORDER_STRONG: Color = rgb((0x4a, 0x4a, 0x4a));
pub const SELECTED_BG: Color = rgb((0x1d, 0x12, 0x30));
pub const TEXT: Color = rgb((0xed, 0xed, 0xed));
pub const MUTED: Color = rgb((0xa1, 0xa1, 0xa1));
pub const META: Color = rgb((0x6b, 0x6b, 0x6b));
pub const SUCCESS: Color = rgb((0x4a, 0xd6, 0x8a));
pub const WARN: Color = rgb((0xf2, 0xb8, 0x4b));
pub const ERROR: Color = rgb((0xf2, 0x6d, 0x6d));

pub fn brand() -> Style {
    Style::new().fg(PURPLE)
}

pub fn accent() -> Style {
    Style::new().fg(LIGHT)
}

pub fn text() -> Style {
    Style::new().fg(TEXT)
}

pub fn strong() -> Style {
    text().add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

pub fn meta() -> Style {
    Style::new().fg(META)
}

pub fn success() -> Style {
    Style::new().fg(SUCCESS)
}

pub fn warn() -> Style {
    Style::new().fg(WARN)
}

pub fn error() -> Style {
    Style::new().fg(ERROR)
}

pub fn selected() -> Style {
    Style::new().bg(SELECTED_BG).fg(TEXT)
}
