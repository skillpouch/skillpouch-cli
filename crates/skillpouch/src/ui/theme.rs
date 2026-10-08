//! Colours and text styles. Truecolor terminals get the brand purple; others
//! the nearest 256-colour shade. `NO_COLOR` and pipes turn colour off.

use console::{Style, StyledObject, style};
use std::fmt::Display;

/// Brand purple, lighter shade (readable on black), darker shade.
const ACCENT: (u8, u8, u8) = (0x84, 0x42, 0xd9);
const ACCENT_LIGHT: (u8, u8, u8) = (0xb3, 0x8b, 0xf0);
const ACCENT_DARK: (u8, u8, u8) = (0x5b, 0x2b, 0xa8);

fn rgb(color: (u8, u8, u8), fallback: u8, stderr: bool) -> Style {
    let truecolor = if stderr {
        console::true_colors_enabled_stderr()
    } else {
        console::true_colors_enabled()
    };
    let base = if stderr {
        Style::new().for_stderr()
    } else {
        Style::new()
    };
    if truecolor {
        base.true_color(color.0, color.1, color.2)
    } else {
        base.color256(fallback)
    }
}

/// The logo's purple.
pub fn brand() -> Style {
    rgb(ACCENT, 98, false)
}

/// The three purples, from light to dark, for gradients.
pub fn shades(stderr: bool) -> [Style; 3] {
    [
        rgb(ACCENT_LIGHT, 141, stderr),
        rgb(ACCENT, 98, stderr),
        rgb(ACCENT_DARK, 55, stderr),
    ]
}

pub fn accent<D: Display>(value: D) -> StyledObject<D> {
    rgb(ACCENT_LIGHT, 141, false).apply_to(value)
}

pub fn accent_err<D: Display>(value: D) -> StyledObject<D> {
    rgb(ACCENT_LIGHT, 141, true).apply_to(value)
}

pub fn heading<D: Display>(value: D) -> StyledObject<D> {
    style(value).bold()
}

pub fn dim<D: Display>(value: D) -> StyledObject<D> {
    style(value).dim()
}

pub fn success<D: Display>(value: D) -> StyledObject<D> {
    style(value).green()
}

pub fn warn<D: Display>(value: D) -> StyledObject<D> {
    style(value).yellow()
}

pub fn error<D: Display>(value: D) -> StyledObject<D> {
    style(value).for_stderr().red().bold()
}

/// Text with `backtick` spans shown as commands: accent colour, no backticks.
pub fn rich(text: &str, stderr: bool) -> String {
    let command = if stderr {
        rgb(ACCENT_LIGHT, 141, true).bold()
    } else {
        rgb(ACCENT_LIGHT, 141, false).bold()
    };
    text.split('`')
        .enumerate()
        .map(|(index, part)| {
            if index % 2 == 1 {
                command.apply_to(part).to_string()
            } else {
                part.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::rich;

    #[test]
    fn commands_lose_their_backticks() {
        console::set_colors_enabled(false);
        assert_eq!(
            rich("Run `skillpouch login` first.", false),
            "Run skillpouch login first."
        );
        assert_eq!(rich("no commands", false), "no commands");
    }
}
