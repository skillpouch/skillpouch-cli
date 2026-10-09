//! What a pouch and this computer agreed on after the last sync, per item:
//! `state/sync/<pouch_id>.json`. A side changed an item when it no longer
//! matches this.

use crate::Result;
use serde::{Deserialize, Serialize};
use sp_core::Home;
use sp_core::home::{read_optional, write_private};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agreed {
    pub artifact_id: String,
    pub revision_id: String,
    pub kind: String,
    pub name: String,
    /// `fingerprint` of the files both sides had.
    pub fingerprint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PouchState {
    #[serde(default)]
    pub epoch: String,
    /// Events up to here are in the pouch folder.
    #[serde(default)]
    pub cursor: u64,
    /// By item key (`skill/pdf`).
    #[serde(default)]
    pub items: BTreeMap<String, Agreed>,
}

impl PouchState {
    fn path(home: &Home, pouch_id: &str) -> PathBuf {
        home.state_dir()
            .join("sync")
            .join(format!("{pouch_id}.json"))
    }

    pub fn load(home: &Home, pouch_id: &str) -> Result<Self> {
        let path = Self::path(home, pouch_id);
        Ok(read_optional(&path)?
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default())
    }

    pub fn save(&self, home: &Home, pouch_id: &str) -> Result<()> {
        let path = Self::path(home, pouch_id);
        if let Some(dir) = path.parent() {
            home.ensure_private_dir(dir)?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| crate::SyncError::Invalid(error.to_string()))?;
        Ok(write_private(&path, &bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_starts_empty() -> Result<()> {
        let dir = tempfile::tempdir().map_err(|e| crate::SyncError::Invalid(e.to_string()))?;
        let home = Home::new(dir.path());
        assert_eq!(PouchState::load(&home, "p")?, PouchState::default());
        let mut state = PouchState {
            epoch: "e".into(),
            cursor: 4,
            ..PouchState::default()
        };
        state.items.insert(
            "skill/pdf".into(),
            Agreed {
                artifact_id: "a".into(),
                revision_id: "r".into(),
                kind: "skill".into(),
                name: "pdf".into(),
                fingerprint: "f".into(),
            },
        );
        state.save(&home, "p")?;
        assert_eq!(PouchState::load(&home, "p")?, state);
        Ok(())
    }
}
