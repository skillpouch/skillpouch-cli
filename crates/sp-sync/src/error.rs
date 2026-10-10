use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Api(Box<sp_api::ApiError>),
    #[error(transparent)]
    Crypto(#[from] sp_crypto::CryptoError),
    #[error(transparent)]
    Core(#[from] sp_core::CoreError),
    #[error("{0}")]
    Invalid(String),
    /// The server offered a version of an item that this computer already
    /// saw replaced by a newer one.
    #[error(
        "the server offered an older version of {0} than this computer already verified; nothing was changed here"
    )]
    Rollback(String),
    /// The pouch was deleted (or is no longer there at all).
    #[error("the pouch this computer syncs was deleted")]
    PouchDeleted,
}

impl From<sp_api::ApiError> for SyncError {
    fn from(error: sp_api::ApiError) -> Self {
        Self::Api(Box::new(error))
    }
}

impl SyncError {
    /// The API error code, when the server refused.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api(error) => error.code(),
            _ => None,
        }
    }

    /// The server couldn't be reached at all.
    pub fn offline(&self) -> bool {
        matches!(self, Self::Api(error) if matches!(**error, sp_api::ApiError::Network(_)))
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;

pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> SyncError + '_ {
    move |source| SyncError::Io {
        path: path.to_path_buf(),
        source,
    }
}
