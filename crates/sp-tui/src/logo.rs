//! The SkillPouch logo for terminals.
//!
//! Terminals that show pictures get the real logo ([`image`]). The rest get
//! [`mark`]: the logo traced in half-block characters, two pixels per cell,
//! with the outline shaded by how much of each pixel the logo covers, so the
//! curves look soft instead of stepped.

use image::{DynamicImage, GenericImageView, RgbaImage};
use std::sync::OnceLock;

/// The logo's purple.
pub const BRAND: Rgb = Rgb(0x84, 0x42, 0xd9);

/// `logo.svg` rendered at 512×512 with a transparent background.
const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");

/// Pixels the logo covers less than this are left empty.
const MIN_COVERAGE: f32 = 0.12;
/// Pixels covered at least this much get the full colour.
const FULL_COVERAGE: f32 = 0.92;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// This colour over a black background at `alpha`.
    fn over_black(self, alpha: f32) -> Self {
        let scale = |channel: u8| (f32::from(channel) * alpha).round() as u8;
        Self(scale(self.0), scale(self.1), scale(self.2))
    }
}

/// One terminal cell of the traced logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub ch: char,
    /// `None`: the terminal's own colour.
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
}

impl Glyph {
    const EMPTY: Self = Self {
        ch: ' ',
        fg: None,
        bg: None,
    };

    /// The cell for a top and a bottom pixel.
    fn of(top: Option<Rgb>, bottom: Option<Rgb>) -> Self {
        match (top, bottom) {
            (None, None) => Self::EMPTY,
            (Some(top), None) => Self {
                ch: '▀',
                fg: Some(top),
                bg: None,
            },
            (None, Some(bottom)) => Self {
                ch: '▄',
                fg: Some(bottom),
                bg: None,
            },
            (Some(top), Some(bottom)) if top == bottom => Self {
                ch: '█',
                fg: Some(top),
                bg: None,
            },
            (Some(top), Some(bottom)) => Self {
                ch: '▀',
                fg: Some(top),
                bg: Some(bottom),
            },
        }
    }
}

/// The traced logo: rows of cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub rows: Vec<Vec<Glyph>>,
}

impl Mark {
    pub fn width(&self) -> u16 {
        self.rows.first().map_or(0, |row| row.len() as u16)
    }

    pub fn height(&self) -> u16 {
        self.rows.len() as u16
    }
}

/// The logo, cropped to its edges.
pub fn image() -> &'static DynamicImage {
    static LOGO: OnceLock<DynamicImage> = OnceLock::new();
    LOGO.get_or_init(|| {
        let decoded = image::load_from_memory_with_format(LOGO_PNG, image::ImageFormat::Png)
            .map(|logo| logo.to_rgba8())
            .unwrap_or_else(|_| RgbaImage::new(1, 1));
        let (x, y, width, height) = visible_bounds(&decoded);
        DynamicImage::ImageRgba8(decoded).crop_imm(x, y, width, height)
    })
}

/// The logo `columns` cells wide. Rows follow its proportions: a cell is about
/// twice as tall as it is wide, so each half-block pixel is close to square.
/// `smooth` shades the outline (needs 24-bit colour); without it a pixel is
/// either full purple or empty.
pub fn mark(columns: u16, smooth: bool) -> Mark {
    let logo = image();
    let columns = u32::from(columns.max(1));
    let pixel_rows = (columns * logo.height()).div_ceil(logo.width()).max(1);
    let coverage = coverage(logo, columns, pixel_rows);
    let pixel = |x: u32, y: u32| {
        let alpha = if y < pixel_rows {
            coverage[(y * columns + x) as usize]
        } else {
            0.0
        };
        shade(alpha, smooth)
    };
    let rows = (0..pixel_rows.div_ceil(2))
        .map(|row| {
            (0..columns)
                .map(|x| Glyph::of(pixel(x, row * 2), pixel(x, row * 2 + 1)))
                .collect()
        })
        .collect();
    Mark { rows }
}

fn shade(alpha: f32, smooth: bool) -> Option<Rgb> {
    if smooth {
        if alpha < MIN_COVERAGE {
            None
        } else if alpha >= FULL_COVERAGE {
            Some(BRAND)
        } else {
            Some(BRAND.over_black(alpha))
        }
    } else {
        (alpha >= 0.5).then_some(BRAND)
    }
}

/// How much of each target pixel the logo covers, from 0 to 1: the average
/// alpha of the source pixels that fall inside it.
fn coverage(logo: &DynamicImage, columns: u32, rows: u32) -> Vec<f32> {
    let (width, height) = logo.dimensions();
    let mut out = Vec::with_capacity((columns * rows) as usize);
    for y in 0..rows {
        let (top, bottom) = span(y, rows, height);
        for x in 0..columns {
            let (left, right) = span(x, columns, width);
            let mut sum = 0u64;
            for source_y in top..bottom {
                for source_x in left..right {
                    sum += u64::from(logo.get_pixel(source_x, source_y).0[3]);
                }
            }
            let count = u64::from((right - left) * (bottom - top)).max(1);
            out.push(sum as f32 / (count as f32 * 255.0));
        }
    }
    out
}

/// The source pixels `[start, end)` behind target pixel `index` of `parts`.
fn span(index: u32, parts: u32, size: u32) -> (u32, u32) {
    let start = index * size / parts;
    let end = ((index + 1) * size / parts).max(start + 1).min(size);
    (start.min(size.saturating_sub(1)), end)
}

/// The smallest rectangle holding every pixel that isn't fully transparent.
fn visible_bounds(image: &RgbaImage) -> (u32, u32, u32, u32) {
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0[3] > 0 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
    }
    if left > right {
        return (0, 0, image.width(), image.height());
    }
    (left, top, right - left + 1, bottom - top + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_is_cropped_to_its_edges() {
        let logo = image();
        assert!(logo.width() > 100 && logo.width() < 512);
        let corner = logo.get_pixel(0, 0).0[3];
        assert_eq!(corner, 0, "the corners of a pouch are empty");
    }

    #[test]
    fn the_mark_keeps_the_logo_proportions() {
        let mark = mark(20, true);
        assert_eq!(mark.width(), 20);
        let ratio = f32::from(mark.height() * 2) / f32::from(mark.width());
        let logo = image();
        let expected = logo.height() as f32 / logo.width() as f32;
        assert!((ratio - expected).abs() < 0.15, "{ratio} vs {expected}");
        assert!(mark.rows.iter().all(|row| row.len() == 20));
    }

    #[test]
    fn the_body_is_full_purple_and_the_outline_shaded() {
        let mark = mark(24, true);
        let colours: Vec<Rgb> = mark
            .rows
            .iter()
            .flatten()
            .flat_map(|glyph| [glyph.fg, glyph.bg])
            .flatten()
            .collect();
        assert!(colours.contains(&BRAND));
        assert!(
            colours.iter().any(|colour| *colour != BRAND),
            "edges get in-between shades"
        );
        let bottom_middle = mark.rows[mark.rows.len() - 2][12];
        assert_eq!(bottom_middle.ch, '█');
    }

    #[test]
    fn without_smoothing_only_the_brand_colour_is_used() {
        let mark = mark(24, false);
        assert!(
            mark.rows
                .iter()
                .flatten()
                .flat_map(|glyph| [glyph.fg, glyph.bg])
                .flatten()
                .all(|colour| colour == BRAND)
        );
    }

    #[test]
    fn half_blocks_follow_the_pixels() {
        let purple = Some(BRAND);
        let dark = Some(Rgb(1, 2, 3));
        assert_eq!(Glyph::of(None, None).ch, ' ');
        assert_eq!(Glyph::of(purple, None).ch, '▀');
        assert_eq!(Glyph::of(None, purple).ch, '▄');
        assert_eq!(Glyph::of(purple, purple).ch, '█');
        assert_eq!(
            Glyph::of(purple, dark),
            Glyph {
                ch: '▀',
                fg: purple,
                bg: dark
            }
        );
    }
}
