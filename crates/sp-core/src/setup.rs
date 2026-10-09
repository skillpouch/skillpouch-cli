//! What this computer was set up to sync (`state/setup.json`): the pouch
//! and how its first sync starts. Written when setup is saved; the first
//! sync marks it done.

use crate::home::{Home, read_optional, write_private};
use crate::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How the first sync treats the pouch and this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum FirstSync {
    /// An empty pouch, filled from this assistant (or left empty).
    Fill {
        assistant: Option<String>,
    },
    UsePouch,
    Merge {
        assistant: String,
    },
    UseComputer {
        assistant: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupRecord {
    pub format: u32,
    /// The login it was made for: logging in again sets the computer up again.
    pub device_id: String,
    pub pouch_id: String,
    pub first_sync: FirstSync,
    /// RFC 3339.
    pub chosen_at: String,
    /// The first sync has run.
    pub done: bool,
}

impl SetupRecord {
    pub const FORMAT: u32 = 1;

    fn path(home: &Home) -> PathBuf {
        home.state_dir().join("setup.json")
    }

    pub fn load(home: &Home) -> Result<Option<Self>> {
        let path = Self::path(home);
        let Some(bytes) = read_optional(&path)? else {
            return Ok(None);
        };
        let record: Self = serde_json::from_slice(&bytes).map_err(|error| CoreError::Invalid {
            path: path.clone(),
            message: error.to_string(),
        })?;
        Ok((record.format == Self::FORMAT).then_some(record))
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        home.ensure_private_dir(home.root())?;
        home.ensure_private_dir(&home.state_dir())?;
        let path = Self::path(home);
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| CoreError::Invalid {
            path: path.clone(),
            message: error.to_string(),
        })?;
        write_private(&path, &bytes)
    }

    /// Forgets the setup: the computer is set up again from the start.
    pub fn remove(home: &Home) -> Result<bool> {
        crate::home::remove_optional(&Self::path(home))
    }

    /// This login hasn't been set up yet (no record, or one from another login).
    pub fn needed(home: &Home, device_id: &str) -> bool {
        Self::load(home)
            .ok()
            .flatten()
            .is_none_or(|record| record.device_id != device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_reads_back_per_login() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("sp-setup-{}", std::process::id()));
        let home = Home::new(&dir);
        assert!(SetupRecord::needed(&home, "d1"));
        let record = SetupRecord {
            format: SetupRecord::FORMAT,
            device_id: "d1".into(),
            pouch_id: "p1".into(),
            first_sync: FirstSync::Merge {
                assistant: "claude-code".into(),
            },
            chosen_at: "2026-10-09T12:00:00Z".into(),
            done: false,
        };
        record.save(&home)?;
        assert_eq!(SetupRecord::load(&home)?, Some(record));
        assert!(!SetupRecord::needed(&home, "d1"));
        assert!(SetupRecord::needed(&home, "d2"));
        let text = std::fs::read_to_string(dir.join("state/setup.json")).unwrap_or_default();
        assert!(text.contains("\"mode\": \"merge\""), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
