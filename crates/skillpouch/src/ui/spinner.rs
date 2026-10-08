//! A spinner on stderr for work that takes a moment. It draws nothing when
//! stderr isn't a terminal, so pipes and CI logs only get the final lines.

use super::theme::accent_err;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "];

pub struct Spinner {
    bar: ProgressBar,
}

impl Spinner {
    pub fn start(message: impl Into<String>) -> Self {
        Self::styled(message, "{spinner} {msg}")
    }

    /// Shows how long it has been spinning, for waits on the user.
    pub fn with_elapsed(message: impl Into<String>) -> Self {
        Self::styled(message, "{spinner} {msg}  {elapsed:.dim}")
    }

    fn styled(message: impl Into<String>, template: &str) -> Self {
        let bar = ProgressBar::new_spinner();
        let frames: Vec<String> = FRAMES
            .iter()
            .map(|frame| accent_err(frame).to_string())
            .collect();
        let frames: Vec<&str> = frames.iter().map(String::as_str).collect();
        if let Ok(style) = ProgressStyle::with_template(template) {
            bar.set_style(style.tick_strings(&frames));
        }
        bar.set_message(message.into());
        bar.enable_steady_tick(Duration::from_millis(80));
        Self { bar }
    }

    pub fn set_message(&self, message: impl Into<String>) {
        self.bar.set_message(message.into());
    }

    pub fn clear(self) {
        self.bar.finish_and_clear();
    }
}
