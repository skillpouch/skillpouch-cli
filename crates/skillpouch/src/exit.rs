//! Exit codes: 0 ok, 1 error, 2 usage (clap), 3 not logged in, 130 cancelled.

use crate::ui;
use std::fmt;
use std::process::ExitCode;

pub const FAILURE: u8 = 1;
pub const NOT_LOGGED_IN: u8 = 3;
pub const CANCELLED: u8 = 130;

/// An error that already explains itself, says what to do next and carries
/// its own exit code.
#[derive(Debug)]
pub struct Exit {
    pub code: u8,
    pub message: String,
    pub hint: Option<String>,
}

impl Exit {
    pub fn with_code(code: u8, message: impl Into<String>, hint: Option<&str>) -> anyhow::Error {
        Self {
            code,
            message: message.into(),
            hint: hint.map(str::to_string),
        }
        .into()
    }

    pub fn failure(message: impl Into<String>, hint: &str) -> anyhow::Error {
        Self::with_code(FAILURE, message, Some(hint))
    }

    pub fn not_logged_in(message: impl Into<String>, hint: &str) -> anyhow::Error {
        Self::with_code(NOT_LOGGED_IN, message, Some(hint))
    }
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Exit {}

pub fn report(error: &anyhow::Error) -> ExitCode {
    if let Some(exit) = error.downcast_ref::<Exit>() {
        ui::error(&exit.message, exit.hint.as_deref());
        return ExitCode::from(exit.code);
    }
    let mut causes = error.chain().map(ToString::to_string);
    let first = causes.next().unwrap_or_default();
    let rest: Vec<String> = causes.collect();
    if rest.is_empty() {
        ui::error(first, None);
    } else {
        ui::error(first, Some(&rest.join(": ")));
    }
    ExitCode::FAILURE
}
