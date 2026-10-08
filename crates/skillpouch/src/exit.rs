//! Exit codes: 0 ok, 1 error, 2 usage (clap), 3 not logged in, 130 cancelled.

use std::fmt;
use std::process::ExitCode;

pub const NOT_LOGGED_IN: u8 = 3;
pub const CANCELLED: u8 = 130;

/// An error that already explains itself and carries its own exit code.
#[derive(Debug)]
pub struct Exit {
    pub code: u8,
    pub message: String,
}

impl Exit {
    pub fn not_logged_in(message: impl Into<String>) -> anyhow::Error {
        Self {
            code: NOT_LOGGED_IN,
            message: message.into(),
        }
        .into()
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
        eprintln!("{}", exit.message);
        return ExitCode::from(exit.code);
    }
    eprintln!("error: {error:#}");
    ExitCode::FAILURE
}
