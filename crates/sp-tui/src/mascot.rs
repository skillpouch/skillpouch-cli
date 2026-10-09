//! The pouch, alive: it blinks, looks around, squishes, hops and wiggles its
//! bow. The ASCII mark is read back into pixels (two per cell), posed, and
//! drawn again with half blocks, so every move keeps the logo's shape.
//!
//! Frames are a pure function of the tick (one tick per redraw, about 80 ms),
//! so the animation needs no state and is easy to test.

use crate::logo::{BRAND, MARK};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Source size in pixels.
const SOURCE_W: i32 = 20;
const SOURCE_H: i32 = 18;
/// Canvas size in pixels: room to stretch sideways and hop.
const CANVAS_W: i32 = 26;
const CANVAS_H: i32 = 24;
/// Rows of the bow, which wiggles on its own.
const BOW_ROWS: i32 = 4;
/// Eyes: top row and the left column of each (3 × 3 pixels).
const EYE_TOP: i32 = 9;
const EYE_LEFT: [i32; 2] = [5, 12];

pub const WIDTH: u16 = CANVAS_W as u16;
pub const HEIGHT: u16 = (CANVAS_H / 2) as u16;

/// One animation loop, in ticks (about 13 s).
const CYCLE: usize = 160;

const PURPLE: Color = Color::Rgb(BRAND.0, BRAND.1, BRAND.2);
const DARK: Color = Color::Rgb(0x0b, 0x07, 0x12);
const SHINE: Color = Color::Rgb(0xf4, 0xee, 0xff);

/// What the pouch is up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Waiting for the user: plays the whole repertoire.
    Idle,
    /// Something is happening (login running): watches, hops now and then.
    Busy,
    /// Something went wrong: slumps, looks down.
    Sad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eyes {
    /// Open; `x`/`y`: where they look, in pixels.
    Open {
        x: i32,
        y: i32,
    },
    Closed,
    /// `> <`, squeezed shut.
    Squeeze,
    /// `^ ^`
    Happy,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub scale_x: f32,
    pub scale_y: f32,
    /// Pixels up.
    pub lift: f32,
    /// Bow offset in pixels.
    pub bow: i32,
    pub eyes: Eyes,
    pub smile: bool,
}

impl Pose {
    const REST: Pose = Pose {
        scale_x: 1.0,
        scale_y: 1.0,
        lift: 0.0,
        bow: 0,
        eyes: Eyes::Open { x: 0, y: 0 },
        smile: false,
    };
}

/// `(tick, scale_x, scale_y, lift)` keyframes, interpolated linearly.
type Keys = &'static [(usize, f32, f32, f32)];

const SQUISH: Keys = &[
    (0, 1.0, 1.0, 0.0),
    (3, 1.16, 0.8, 0.0),
    (6, 0.9, 1.12, 0.0),
    (9, 1.06, 0.94, 0.0),
    (12, 0.98, 1.02, 0.0),
    (15, 1.0, 1.0, 0.0),
];

const HOP: Keys = &[
    (0, 1.0, 1.0, 0.0),
    (3, 1.14, 0.84, 0.0),
    (5, 0.9, 1.14, 2.0),
    (8, 0.96, 1.05, 5.0),
    (10, 1.0, 1.0, 5.5),
    (12, 0.96, 1.05, 3.5),
    (14, 1.18, 0.8, 0.0),
    (16, 0.96, 1.04, 0.0),
    (18, 1.0, 1.0, 0.0),
];

fn keyframes(keys: Keys, t: usize) -> (f32, f32, f32) {
    let mut previous = keys[0];
    for &key in keys {
        if t <= key.0 {
            let span = (key.0 - previous.0).max(1) as f32;
            let k = (t.saturating_sub(previous.0)) as f32 / span;
            let lerp = |a: f32, b: f32| a + (b - a) * k;
            return (
                lerp(previous.1, key.1),
                lerp(previous.2, key.2),
                lerp(previous.3, key.3),
            );
        }
        previous = key;
    }
    (previous.1, previous.2, previous.3)
}

fn within(t: usize, start: usize, len: usize) -> Option<usize> {
    (start..start + len).contains(&t).then(|| t - start)
}

/// The pose at `tick`.
pub fn pose(tick: usize, mood: Mood) -> Pose {
    let t = tick % CYCLE;
    let mut pose = Pose::REST;
    match mood {
        Mood::Sad => {
            pose.scale_y = 0.93;
            pose.scale_x = 1.04;
            pose.eyes = if within(t % 80, 50, 3).is_some() {
                Eyes::Closed
            } else {
                Eyes::Open { x: 0, y: 1 }
            };
            return pose;
        }
        Mood::Busy => {
            if let Some(s) = within(t, 60, 18) {
                (pose.scale_x, pose.scale_y, pose.lift) = keyframes(HOP, s);
            }
            pose.eyes = match t {
                20..=22 | 100..=102 => Eyes::Closed,
                30..=50 => Eyes::Open { x: 2, y: 0 },
                110..=130 => Eyes::Open { x: -2, y: 0 },
                63..=64 | 74..=75 => Eyes::Squeeze,
                65..=72 => Eyes::Happy,
                _ => Eyes::Open { x: 0, y: 0 },
            };
            return pose;
        }
        Mood::Idle => {}
    }

    // Breathing, under everything else.
    let breath = (t as f32 / 40.0 * std::f32::consts::TAU).sin();
    pose.scale_y = 1.0 + 0.03 * breath;

    if let Some(s) = within(t, 75, 16) {
        (pose.scale_x, pose.scale_y, pose.lift) = keyframes(SQUISH, s);
    }
    if let Some(s) = within(t, 100, 19) {
        (pose.scale_x, pose.scale_y, pose.lift) = keyframes(HOP, s);
    }
    if let Some(s) = within(t, 124, 14) {
        pose.bow = if (s / 2) % 2 == 0 { 1 } else { -1 };
    }
    pose.eyes = match t {
        18..=20 | 62..=63 => Eyes::Closed,
        32..=43 => Eyes::Open { x: -2, y: 0 },
        44..=55 => Eyes::Open { x: 2, y: 0 },
        // Squeezed shut while squashed, happy in the air.
        76..=78 | 102..=103 | 113..=115 => Eyes::Squeeze,
        104..=112 | 140..=159 => Eyes::Happy,
        _ => Eyes::Open { x: 0, y: 0 },
    };
    pose.smile = (140..CYCLE).contains(&t) || (104..=112).contains(&t);
    if let Some(s) = within(t, 140, 20) {
        pose.lift = if (s / 3) % 2 == 0 { 0.0 } else { 1.0 };
    }
    pose
}

/// The mark as pixels: `true` where the pouch is.
fn body(x: i32, y: i32) -> bool {
    if !(0..SOURCE_W).contains(&x) || !(0..SOURCE_H).contains(&y) {
        return false;
    }
    let Some(row) = MARK.get((y / 2) as usize) else {
        return false;
    };
    match row.chars().nth(x as usize) {
        Some('█') => true,
        Some('▀') => y % 2 == 0,
        Some('▄') => y % 2 == 1,
        _ => false,
    }
}

/// The face, over the body: `None` keeps the body colour.
fn face(x: i32, y: i32, pose: &Pose) -> Option<Color> {
    for left in EYE_LEFT {
        let (look_x, look_y) = match pose.eyes {
            Eyes::Open { x, y } => (x, y),
            _ => (0, 0),
        };
        let (ex, ey) = (x - left - look_x, y - EYE_TOP - look_y);
        if !(0..3).contains(&ex) || !(0..3).contains(&ey) {
            continue;
        }
        return match pose.eyes {
            Eyes::Open { .. } if (ex, ey) == (0, 0) => Some(SHINE),
            Eyes::Open { .. } => Some(DARK),
            Eyes::Closed => (ey == 1).then_some(DARK),
            Eyes::Happy => matches!((ex, ey), (1, 0) | (0, 1) | (2, 1)).then_some(DARK),
            // `>` on the left, `<` on the right.
            Eyes::Squeeze => {
                let tip = if left == EYE_LEFT[0] { 2 } else { 0 };
                (if ey == 1 { ex == tip } else { ex == 2 - tip }).then_some(DARK)
            }
        };
    }
    if pose.smile && matches!((x, y), (8, 13) | (11, 13) | (9, 14) | (10, 14)) {
        return Some(DARK);
    }
    None
}

/// The colour of canvas pixel `(x, y)` in `pose`.
fn pixel(x: i32, y: i32, pose: &Pose) -> Option<Color> {
    // Scale around the bottom centre, so squashes stay on the ground.
    let cx = x as f32 + 0.5 - CANVAS_W as f32 / 2.0;
    let cy = y as f32 + 0.5 - (CANVAS_H as f32 - pose.lift.round());
    let sy = (cy / pose.scale_y + SOURCE_H as f32).floor() as i32;
    let mut sx = (cx / pose.scale_x + SOURCE_W as f32 / 2.0).floor() as i32;
    if sy < BOW_ROWS {
        sx -= pose.bow;
    }
    if !body(sx, sy) {
        return None;
    }
    Some(face(sx, sy, pose).unwrap_or(PURPLE))
}

/// One terminal cell of the pouch: a half block and its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
}

pub type Rgb = (u8, u8, u8);

fn rgb(color: Color) -> Rgb {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => BRAND,
    }
}

/// The pose as `HEIGHT` rows of `WIDTH` cells, for drawing outside ratatui.
pub fn cells(pose: &Pose) -> Vec<Vec<Cell>> {
    (0..CANVAS_H / 2)
        .map(|row| {
            (0..CANVAS_W)
                .map(|x| {
                    let top = pixel(x, row * 2, pose);
                    let bottom = pixel(x, row * 2 + 1, pose);
                    let (ch, fg, bg) = match (top, bottom) {
                        (None, None) => (' ', None, None),
                        (Some(top), None) => ('▀', Some(rgb(top)), None),
                        (None, Some(bottom)) => ('▄', Some(rgb(bottom)), None),
                        (Some(top), Some(bottom)) if top == bottom => ('█', Some(rgb(top)), None),
                        (Some(top), Some(bottom)) => ('▀', Some(rgb(top)), Some(rgb(bottom))),
                    };
                    Cell { ch, fg, bg }
                })
                .collect()
        })
        .collect()
}

/// The pose drawn as `HEIGHT` lines of `WIDTH` cells.
pub fn render(pose: &Pose) -> Vec<Line<'static>> {
    cells(pose)
        .into_iter()
        .map(|row| {
            let spans: Vec<Span> = row
                .into_iter()
                .map(|cell| {
                    let mut style = Style::new();
                    if let Some((r, g, b)) = cell.fg {
                        style = style.fg(Color::Rgb(r, g, b));
                    }
                    if let Some((r, g, b)) = cell.bg {
                        style = style.bg(Color::Rgb(r, g, b));
                    }
                    if cell.fg.is_none() {
                        Span::raw(" ")
                    } else {
                        Span::styled(cell.ch.to_string(), style)
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(pose: &Pose) -> Vec<Vec<Option<Color>>> {
        (0..CANVAS_H)
            .map(|y| (0..CANVAS_W).map(|x| pixel(x, y, pose)).collect())
            .collect()
    }

    fn count(pose: &Pose, colour: Color) -> usize {
        pixels(pose)
            .iter()
            .flatten()
            .filter(|p| **p == Some(colour))
            .count()
    }

    fn top_row(pose: &Pose) -> usize {
        pixels(pose)
            .iter()
            .position(|row| row.iter().any(Option::is_some))
            .unwrap_or(usize::MAX)
    }

    #[test]
    fn at_rest_it_is_the_logo_with_eyes() {
        let rest = Pose::REST;
        let body_pixels = (0..SOURCE_H)
            .flat_map(|y| (0..SOURCE_W).map(move |x| body(x, y)))
            .filter(|b| *b)
            .count();
        let drawn = pixels(&rest)
            .iter()
            .flatten()
            .filter(|p| p.is_some())
            .count();
        assert_eq!(drawn, body_pixels);
        assert_eq!(count(&rest, DARK), 2 * 8);
        assert_eq!(count(&rest, SHINE), 2);
    }

    #[test]
    fn every_frame_fits_the_canvas() {
        for mood in [Mood::Idle, Mood::Busy, Mood::Sad] {
            for tick in 0..CYCLE {
                let lines = render(&pose(tick, mood));
                assert_eq!(lines.len(), usize::from(HEIGHT));
                assert!(lines.iter().all(|line| line.width() == usize::from(WIDTH)));
            }
        }
    }

    #[test]
    fn it_blinks_hops_and_looks_around() {
        let poses: Vec<Pose> = (0..CYCLE).map(|t| pose(t, Mood::Idle)).collect();
        assert!(poses.iter().any(|p| p.eyes == Eyes::Closed));
        assert!(poses.iter().any(|p| p.eyes == Eyes::Happy && p.smile));
        assert!(poses.iter().any(|p| p.eyes == Eyes::Open { x: -2, y: 0 }));
        assert!(poses.iter().any(|p| p.eyes == Eyes::Squeeze));
        assert!(poses.iter().any(|p| p.bow != 0));
        // In the air, the top of the pouch is higher than at rest.
        let highest = poses.iter().map(top_row).min().unwrap_or(usize::MAX);
        assert!(highest + 4 <= top_row(&Pose::REST));
        // Squashed, it is wider than at rest.
        let squashed = pose(78, Mood::Idle);
        assert!(count(&squashed, PURPLE) > 0 && squashed.scale_x > 1.1);
    }

    #[test]
    fn a_closed_eye_is_a_line_and_happy_eyes_are_arches() {
        let closed = Pose {
            eyes: Eyes::Closed,
            ..Pose::REST
        };
        assert_eq!(count(&closed, DARK), 2 * 3);
        assert_eq!(count(&closed, SHINE), 0);
        let happy = Pose {
            eyes: Eyes::Happy,
            smile: true,
            ..Pose::REST
        };
        assert_eq!(count(&happy, DARK), 2 * 3 + 4);
        let squeezed = Pose {
            eyes: Eyes::Squeeze,
            ..Pose::REST
        };
        assert_eq!(count(&squeezed, DARK), 2 * 3);
    }

    #[test]
    fn sad_pouches_slump() {
        let sad = pose(0, Mood::Sad);
        assert!(top_row(&sad) > top_row(&Pose::REST));
    }
}
