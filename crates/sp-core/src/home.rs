//! The `~/.skillpouch` folder (or `$SKILLPOUCH_HOME`).

use crate::{CoreError, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Home {
    root: PathBuf,
}

impl Home {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$SKILLPOUCH_HOME`, else `~/.skillpouch`.
    pub fn locate() -> Result<Self> {
        match std::env::var_os("SKILLPOUCH_HOME") {
            Some(path) if !path.is_empty() => Ok(Self::new(path)),
            _ => dirs::home_dir()
                .map(|home| Self::new(home.join(".skillpouch")))
                .ok_or(CoreError::NoHome),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn credentials_file(&self) -> PathBuf {
        self.state_dir().join("credentials.json")
    }

    /// Creates `dir` (and parents) readable only by this user.
    pub fn ensure_private_dir(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).map_err(|source| io(dir, source))?;
        restrict(dir, 0o700)
    }
}

/// The user's home folder (where assistants keep their settings).
pub fn user_home() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Writes `bytes` to `path` atomically, readable only by this user.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = open_private(&tmp)?;
        file.write_all(bytes).map_err(|source| io(&tmp, source))?;
        file.sync_all().map_err(|source| io(&tmp, source))?;
    }
    fs::rename(&tmp, path).map_err(|source| io(path, source))
}

/// Reads `path`, or `None` if it doesn't exist.
pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io(path, source)),
    }
}

/// Deletes `path`; returns whether it existed.
pub fn remove_optional(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io(path, source)),
    }
}

pub(crate) fn io(path: &Path, source: std::io::Error) -> CoreError {
    CoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(unix)]
fn open_private(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|source| io(path, source))
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> Result<fs::File> {
    fs::File::create(path).map_err(|source| io(path, source))
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| io(path, source))
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_round_trip() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("sp-home-{}", std::process::id()));
        let home = Home::new(&dir);
        home.ensure_private_dir(&home.state_dir())?;
        let path = home.credentials_file();

        assert_eq!(read_optional(&path)?, None);
        write_private(&path, b"secret")?;
        assert_eq!(read_optional(&path)?.as_deref(), Some(&b"secret"[..]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)
                .map_err(|e| io(&path, e))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(remove_optional(&path)?);
        assert!(!remove_optional(&path)?);
        fs::remove_dir_all(&dir).map_err(|e| io(&dir, e))
    }
}
