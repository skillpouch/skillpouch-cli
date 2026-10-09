//! Shared building blocks of the SkillPouch CLI: where it keeps its files,
//! its configuration and the credentials of this computer.

pub mod assistants;
pub mod canonical;
pub mod config;
pub mod credentials;
pub mod home;
pub mod setup;

pub use config::{Config, SyncChoice};
pub use credentials::Credentials;
pub use home::Home;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("could not find your home folder; set SKILLPOUCH_HOME")]
    NoHome,
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Invalid {
        path: std::path::PathBuf,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, CoreError>;
