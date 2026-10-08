//! The SkillPouch pouch in block characters, traced from the logo at two
//! pixels per cell. Shared by the TUI and the CLI banner.

/// The logo's purple.
pub const BRAND: (u8, u8, u8) = (0x84, 0x42, 0xd9);

pub const MARK: [&str; 9] = [
    "    ▄█▀█▄  ▄█▀█▄    ",
    "     ▀█▄█▀██▄█▀     ",
    "     ██████████     ",
    "   ▄████████████▄   ",
    " ▄████████████████▄ ",
    "████████████████████",
    "████████████████████",
    "▀██████████████████▀",
    "  ▀▀████████████▀▀  ",
];

pub const MARK_WIDTH: u16 = 20;
pub const MARK_HEIGHT: u16 = MARK.len() as u16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_as_wide_as_the_mark() {
        for row in MARK {
            assert_eq!(row.chars().count(), usize::from(MARK_WIDTH), "{row}");
        }
    }
}
