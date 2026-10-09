//! The banner with the pouch alive beside it: it hops and looks around while
//! the CLI waits, then smiles or slumps when it is done. The whole block is
//! redrawn in place, so nothing else may print to stdout while it runs.
//! Outside a terminal it prints once, without animation.

use super::theme::{self, dim, heading};
use console::{Style, Term};
use sp_tui::mascot::{self, Cell, Eyes, Mood, Pose, Rgb};
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(80);
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Text-column rows: name, tagline, encryption, then the status.
const TITLE_ROW: usize = 3;
const STATUS_ROW: usize = 7;
const GAP: &str = "   ";

/// How the pouch looks once the wait is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Happy,
    Sad,
}

#[derive(Default)]
struct State {
    status: String,
    notes: Vec<String>,
    details: Vec<String>,
    since: Option<Instant>,
    tick: usize,
    drawn: usize,
    stop: bool,
}

pub struct LiveBanner {
    version: String,
    state: Arc<Mutex<State>>,
    thread: Option<JoinHandle<()>>,
    live: bool,
}

impl LiveBanner {
    pub fn start(version: &str, status: impl Into<String>) -> Self {
        let live = Term::stdout().is_term() && Term::stderr().is_term();
        let state = Arc::new(Mutex::new(State {
            status: status.into(),
            since: Some(Instant::now()),
            ..State::default()
        }));
        let mut banner = Self {
            version: version.to_string(),
            state,
            thread: None,
            live,
        };
        if live {
            let _ = Term::stdout().hide_cursor();
            println!();
            banner.draw(mascot::pose(0, Mood::Busy));
            let version = banner.version.clone();
            let state = Arc::clone(&banner.state);
            banner.thread = Some(std::thread::spawn(move || {
                loop {
                    std::thread::sleep(FRAME);
                    let mut state = lock(&state);
                    if state.stop {
                        break;
                    }
                    state.tick += 1;
                    let pose = mascot::pose(state.tick, Mood::Busy);
                    redraw(&version, &mut state, pose, true);
                }
            }));
        } else {
            super::banner(version);
        }
        banner
    }

    /// What the CLI is waiting for, with a spinner and the time it has taken.
    pub fn status(&self, status: impl Into<String>) {
        let mut state = lock(&self.state);
        state.status = status.into();
        state.since = Some(Instant::now());
        if !self.live {
            println!("{}", state.status);
        }
    }

    /// A line under the status that stays until the end.
    pub fn note(&self, note: impl Into<String>) {
        let note = note.into();
        if !self.live {
            println!("{note}");
        }
        lock(&self.state).notes.push(note);
    }

    /// Lines under the status, e.g. a link to open.
    pub fn details(&self, details: Vec<String>) {
        let mut state = lock(&self.state);
        if !self.live {
            for line in &details {
                println!("{line}");
            }
        }
        state.details = details;
    }

    /// Stops the animation and leaves the last frame on screen, without the
    /// status, notes and details.
    pub fn finish(mut self, ending: Ending) {
        self.stop(ending);
    }

    fn stop(&mut self, ending: Ending) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        lock(&self.state).stop = true;
        let _ = thread.join();
        let mut state = lock(&self.state);
        state.status.clear();
        state.notes.clear();
        state.details.clear();
        state.since = None;
        let tick = state.tick;
        redraw(&self.version, &mut state, ending.pose(tick), false);
        drop(state);
        println!();
        let _ = Term::stdout().show_cursor();
    }

    fn draw(&self, pose: Pose) {
        redraw(&self.version, &mut lock(&self.state), pose, true);
    }
}

impl Drop for LiveBanner {
    fn drop(&mut self) {
        self.stop(Ending::Sad);
    }
}

impl Ending {
    fn pose(self, tick: usize) -> Pose {
        match self {
            Ending::Happy => Pose {
                eyes: Eyes::Happy,
                smile: true,
                ..mascot::pose(0, Mood::Idle)
            },
            Ending::Sad => mascot::pose(tick, Mood::Sad),
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn redraw(version: &str, state: &mut State, pose: Pose, spinning: bool) {
    let columns = super::width().max(1);
    let lines = frame(
        version,
        state,
        &pose,
        spinning,
        columns,
        console::true_colors_enabled(),
    );
    let mut out = String::new();
    if state.drawn > 0 {
        let _ = write!(out, "\x1b[{}A\r\x1b[J", state.drawn);
    }
    let mut rows = 0;
    for line in &lines {
        let _ = writeln!(out, "{line}");
        rows += console::measure_text_width(line).saturating_sub(1) / columns + 1;
    }
    state.drawn = rows;
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}

/// The block's lines: the pouch with the text column beside it. Details too
/// wide for the column go below in full, so links stay whole.
fn frame(
    version: &str,
    state: &State,
    pose: &Pose,
    spinning: bool,
    columns: usize,
    truecolor: bool,
) -> Vec<String> {
    let mut text = vec![String::new(); usize::from(mascot::HEIGHT)];
    text[TITLE_ROW] = format!("{} {}", heading("SkillPouch"), dim(version));
    text[TITLE_ROW + 1] = "Your skills, agents and MCP servers on every computer.".to_string();
    text[TITLE_ROW + 2] = dim("End-to-end encrypted.").to_string();
    if !state.status.is_empty() {
        let spinner = if spinning {
            SPINNER[state.tick % SPINNER.len()]
        } else {
            " "
        };
        let elapsed = state
            .since
            .map(|since| since.elapsed().as_secs())
            .filter(|seconds| *seconds > 0)
            .map(|seconds| dim(format!("  {seconds}s")).to_string())
            .unwrap_or_default();
        text[STATUS_ROW] = format!("{}  {}{elapsed}", theme::accent(spinner), state.status);
    }
    let column = 2 + usize::from(mascot::WIDTH) + GAP.len();
    let mut row = STATUS_ROW + 2;
    let mut below = Vec::new();
    for detail in state.details.iter().chain(&state.notes) {
        let fits = column + console::measure_text_width(detail) < columns;
        match text.get_mut(row).filter(|_| fits && below.is_empty()) {
            Some(slot) => {
                *slot = detail.clone();
                row += 1;
            }
            None => below.push(detail.clone()),
        }
    }
    let mut lines: Vec<String> = mascot::cells(pose)
        .iter()
        .zip(text)
        .map(|(row, text)| {
            let line = format!("  {}{GAP}{text}", paint(row, truecolor));
            console::truncate_str(line.trim_end(), columns.saturating_sub(1), "…").into_owned()
        })
        .collect();
    if !below.is_empty() {
        lines.push(String::new());
        lines.extend(below.into_iter().map(|line| format!("  {line}")));
    }
    lines
}

fn paint(row: &[Cell], truecolor: bool) -> String {
    row.iter()
        .map(|cell| {
            let Some(fg) = cell.fg else {
                return " ".to_string();
            };
            let mut style = colour(Style::new(), fg, truecolor, false);
            if let Some(bg) = cell.bg {
                style = colour(style, bg, truecolor, true);
            }
            style.apply_to(cell.ch).to_string()
        })
        .collect()
}

fn colour(style: Style, (r, g, b): Rgb, truecolor: bool, background: bool) -> Style {
    match (truecolor, background) {
        (true, false) => style.true_color(r, g, b),
        (true, true) => style.on_true_color(r, g, b),
        (false, background) => {
            let shade = nearest_256((r, g, b));
            if background {
                style.on_color256(shade)
            } else {
                style.color256(shade)
            }
        }
    }
}

/// The closest colour in the 6×6×6 cube of the 256-colour palette.
fn nearest_256((r, g, b): Rgb) -> u8 {
    let level = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    16 + 36 * level(r) + 6 * level(g) + level(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| console::strip_ansi_codes(line).to_string())
            .collect()
    }

    #[test]
    fn the_text_sits_beside_the_pouch() {
        let state = State {
            status: "Logging in".into(),
            details: vec!["Open https://example.test".into()],
            ..State::default()
        };
        let lines = plain(&frame(
            "v1",
            &state,
            &mascot::pose(0, Mood::Busy),
            true,
            100,
            true,
        ));
        assert_eq!(lines.len(), usize::from(mascot::HEIGHT));
        assert!(lines[TITLE_ROW].ends_with("SkillPouch v1"));
        assert!(lines[STATUS_ROW].ends_with("⠋  Logging in"));
        assert!(lines[STATUS_ROW + 2].ends_with("Open https://example.test"));
        assert!(lines.iter().any(|line| line.contains('█')));
    }

    #[test]
    fn details_that_do_not_fit_go_below_in_full() {
        let link = format!("https://example.test/{}", "x".repeat(80));
        let state = State {
            details: vec![link.clone(), "short".into()],
            ..State::default()
        };
        let lines = plain(&frame(
            "v1",
            &state,
            &mascot::pose(0, Mood::Busy),
            false,
            80,
            false,
        ));
        assert_eq!(lines.len(), usize::from(mascot::HEIGHT) + 3);
        assert_eq!(lines[lines.len() - 2], format!("  {link}"));
        assert_eq!(lines[lines.len() - 1], "  short");
    }

    #[test]
    fn it_picks_the_nearest_palette_colour() {
        assert_eq!(nearest_256((0, 0, 0)), 16);
        assert_eq!(nearest_256((255, 255, 255)), 231);
        assert_eq!(nearest_256((0x84, 0x42, 0xd9)), 98);
    }
}
