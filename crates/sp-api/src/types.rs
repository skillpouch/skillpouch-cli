//! Request and response bodies, mirroring `@skillpouch/contracts`. Responses
//! ignore fields they don't know, so newer servers keep working.

use serde::{Deserialize, Serialize};
use sp_crypto::PublicJwk;
pub use sp_crypto::vault::{KeyEnvelope, RosterStatement};

#[derive(Debug, Clone, Serialize)]
pub struct CliAuthCreate {
    pub code_challenge: String,
    pub device_name: String,
    pub assistants: Vec<String>,
    pub os: &'static str,
    pub arch: String,
    pub cli_version: String,
    pub sign_pub: PublicJwk,
    pub enc_pub: PublicJwk,
    pub key_storage: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CliAuthCreated {
    pub request_id: String,
    pub authorize_url: String,
    /// Seconds.
    pub expires_in: u64,
    /// Seconds between polls.
    pub interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceTokens {
    pub access_token: String,
    /// Seconds.
    pub expires_in: u64,
    pub refresh_token: String,
    pub device_id: String,
    pub storage_account_id: String,
    /// Only on the first token, when the login is approved.
    #[serde(default)]
    pub member_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CliAuthToken {
    Pending {
        interval: u64,
    },
    Approved {
        #[serde(flatten)]
        tokens: DeviceTokens,
        welcome: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    pub user: MeUser,
    pub account: MeAccount,
    pub device: Option<MeDevice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeUser {
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeAccount {
    pub id: String,
    pub entitlements: Entitlements,
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub delete_after: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entitlements {
    pub plan_slug: String,
    pub plan_name: String,
    pub pouch_limit: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PouchList {
    pub pouches: Vec<PouchSummary>,
}

/// A pouch as the server sees it: its name and items are encrypted.
#[derive(Debug, Clone, Deserialize)]
pub struct PouchSummary {
    pub id: String,
    /// `active`, `locked`, `archived` or `deleted`.
    pub state: String,
    pub event_seq: u64,
    pub created_seq: u64,
    pub delete_after: Option<String>,
    pub created_at: String,
    /// Changes whenever the pouch's history is rewritten; events carry it.
    #[serde(default)]
    pub epoch: String,
    /// The key generation new writes must use.
    #[serde(default)]
    pub key_gen: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceList {
    pub devices: Vec<Device>,
    pub browsers: Vec<Browser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub cli_version: String,
    pub assistants: Vec<String>,
    pub cursors: Vec<Cursor>,
    /// `active`, `suspended` or `revoked`.
    pub state: String,
    pub key_storage: Option<String>,
    pub active_pouch_id: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cursor {
    pub pouch_id: String,
    pub acked_seq: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Browser {
    pub last_seen_at: Option<String>,
    pub revoked_at: Option<String>,
    pub is_current: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactList {
    pub items: Vec<ArtifactHead>,
}

/// An item as the server sees it: its name and files are encrypted.
#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactHead {
    pub id: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Roster {
    pub head_seq: i64,
    pub statements: Vec<RosterStatement>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyEnvelopes {
    pub key_envelopes: Vec<KeyEnvelope>,
}

/// Tokens per UTC day, assistant and model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRow {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub assistant: String,
    pub model: String,
    #[serde(flatten)]
    pub counts: UsageCounts,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageCounts {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageReport {
    pub rows: Vec<UsageRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsageSummary {
    pub days: Vec<UsageRow>,
    pub all_time: UsageCounts,
    pub updated_at: Option<String>,
}

/// An item with its newest revision (sealed: kind, name and files are inside).
#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactRecord {
    pub id: String,
    pub deleted_at: Option<String>,
    pub updated_at: String,
    pub head: Option<RevisionRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevisionRecord {
    pub id: String,
    pub artifact_id: String,
    pub parent_id: Option<String>,
    pub envelope: String,
    pub signature: String,
    pub signer_member_id: String,
    pub key_gen: u32,
    pub blob_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlobCheck {
    pub id: String,
    pub stored_size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlobMissing {
    pub missing: Vec<String>,
}

/// One change to commit. Every field is sent, so the signed digest matches
/// what the server parses.
#[derive(Debug, Clone, Serialize)]
pub struct Operation {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub artifact_id: String,
    pub revision_id: String,
    pub base_revision_id: Option<String>,
    pub merge_parent_id: Option<String>,
    pub name_index: Option<String>,
    pub conflict_id: Option<String>,
    pub key_gen: u32,
    pub envelope: String,
    pub signature: String,
    pub signer_member_id: String,
    pub blob_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitRequest {
    pub operation_id: String,
    pub epoch: String,
    pub ops: Vec<Operation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OperationResult {
    Applied {
        artifact_id: String,
        revision_id: String,
        seq: u64,
    },
    Conflict {
        artifact_id: String,
        revision_id: String,
        conflict_type: String,
        server_revision_id: Option<String>,
        seq: u64,
    },
    Rejected {
        artifact_id: String,
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitResponse {
    pub results: Vec<OperationResult>,
    pub high_water: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PouchEvent {
    pub seq: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub artifact_id: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventPage {
    pub events: Vec<PouchEvent>,
    pub next_cursor: u64,
    pub high_water: u64,
    pub epoch: String,
}

/// Two versions of an item the server keeps until someone picks one.
#[derive(Debug, Clone, Deserialize)]
pub struct ConflictRecord {
    pub id: String,
    pub artifact_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConflictList {
    pub conflicts: Vec<ConflictRecord>,
}
