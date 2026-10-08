//! Request and response bodies, mirroring `@skillpouch/contracts`. Responses
//! ignore fields they don't know, so newer servers keep working.

use serde::{Deserialize, Serialize};
use sp_crypto::PublicJwk;

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
