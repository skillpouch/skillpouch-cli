//! What a pouch and this computer agreed on after the last sync, per item:
//! `state/sync/<pouch_id>.json`. A side changed an item when it no longer
//! matches this.

use crate::Result;
use serde::{Deserialize, Serialize};
use sp_core::Home;
use sp_core::home::{read_optional, write_private};
use std::collections::{BTreeMap, BTreeSet};
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

/// The newest head this computer verified for one item, and every revision
/// it knows came before. A revision that was replaced never becomes the head
/// again (the server only moves a head to a new revision), so a server that
/// offers one of them is replaying an older version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub head: String,
    #[serde(default)]
    pub older: BTreeSet<String>,
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
    /// By artifact id. Never forgotten, not even when the pouch's history
    /// looks rewritten: that is when a rollback would slip through.
    #[serde(default)]
    pub seen: BTreeMap<String, Seen>,
}

impl PouchState {
    /// Starts over on everything but what was seen.
    pub fn restart(&mut self) {
        *self = Self {
            seen: std::mem::take(&mut self.seen),
            ..Self::default()
        };
    }

    /// The artifact's head the server offers is one this computer saw
    /// replaced.
    pub fn is_rollback(&self, artifact_id: &str, revision_id: &str) -> bool {
        self.seen
            .get(artifact_id)
            .is_some_and(|seen| seen.older.contains(revision_id))
    }

    /// Remembers `revision_id` as the artifact's newest verified head;
    /// `parents` (from its signed body) came before it.
    pub fn saw<'a>(
        &mut self,
        artifact_id: &str,
        revision_id: &str,
        parents: impl IntoIterator<Item = &'a str>,
    ) {
        let seen = self.seen.entry(artifact_id.to_string()).or_default();
        if seen.head != revision_id {
            let previous = std::mem::replace(&mut seen.head, revision_id.to_string());
            if !previous.is_empty() {
                seen.older.insert(previous);
            }
        }
        seen.older.extend(
            parents
                .into_iter()
                .filter(|parent| *parent != revision_id)
                .map(str::to_string),
        );
    }
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

/// The newest account roster this computer verified: `state/roster.json`.
/// The server may not show it an older or a different one later.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterPin {
    pub genesis_hash: String,
    pub member_id: String,
    pub head_seq: u64,
    pub head_hash: String,
}

impl RosterPin {
    fn path(home: &Home) -> PathBuf {
        home.state_dir().join("roster.json")
    }

    pub fn load(home: &Home) -> Result<Option<Self>> {
        Ok(read_optional(&Self::path(home))?.and_then(|bytes| serde_json::from_slice(&bytes).ok()))
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        home.ensure_private_dir(&home.state_dir())?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| crate::SyncError::Invalid(error.to_string()))?;
        Ok(write_private(&Self::path(home), &bytes)?)
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
        state.saw("a", "r", ["q"]);
        state.save(&home, "p")?;
        assert_eq!(PouchState::load(&home, "p")?, state);
        Ok(())
    }

    #[test]
    fn replaced_heads_are_remembered_across_a_restart() {
        let mut state = PouchState::default();
        state.saw("a", "r1", []);
        assert!(!state.is_rollback("a", "r1"));
        state.saw("a", "r2", ["r1"]);
        state.saw("a", "r3", ["r2", "m1"]);
        for older in ["r1", "r2", "m1"] {
            assert!(state.is_rollback("a", older), "{older}");
        }
        assert!(!state.is_rollback("a", "r3"));
        assert!(!state.is_rollback("a", "r4"), "a newer head");
        assert!(!state.is_rollback("b", "r1"), "another item");

        state.cursor = 9;
        state.restart();
        assert_eq!((state.cursor, state.items.len()), (0, 0));
        assert!(state.is_rollback("a", "r2"));
    }
}
