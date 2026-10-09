//! How syncing is going, for the dashboard: `state/sync-status.json`,
//! written by whichever process syncs.

use crate::Result;
use serde::{Deserialize, Serialize};
use sp_core::Home;
use sp_core::home::{read_optional, write_private};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Up to date, waiting for changes.
    Idle,
    Syncing,
    /// The server can't be reached; changes wait on this computer.
    Offline,
    /// The last attempt failed (`error` says why).
    Failed,
    /// Nothing to sync: no pouch chosen, or the pouch is locked.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub phase: Phase,
    pub pouch_id: Option<String>,
    /// RFC 3339 of the last sync that finished without error.
    pub last_synced_at: Option<String>,
    /// Why the last attempt failed or why syncing is stopped.
    pub message: Option<String>,
    /// Items sent and received by the last sync that changed anything.
    #[serde(default)]
    pub sent: usize,
    #[serde(default)]
    pub received: usize,
    /// Items both sides changed; this computer's copy was kept as `<name>-conflict`.
    #[serde(default)]
    pub conflicts: usize,
    /// The background process keeping this up to date.
    pub daemon_pid: Option<u32>,
    pub updated_at: String,
}

impl SyncStatus {
    fn path(home: &Home) -> PathBuf {
        home.state_dir().join("sync-status.json")
    }

    pub fn load(home: &Home) -> Option<Self> {
        read_optional(&Self::path(home))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        home.ensure_private_dir(&home.state_dir())?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| crate::SyncError::Invalid(error.to_string()))?;
        Ok(write_private(&Self::path(home), &bytes)?)
    }
}
