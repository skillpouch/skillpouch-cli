//! `state/credentials.json`: this computer's device keys and tokens.
//!
//! The file is `0600` inside a `0700` folder (`key_storage: file`). OS
//! keystores and hardware keys come later.

use crate::home::{read_optional, remove_optional, write_private};
use crate::{CoreError, Home, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub format: u32,
    /// The server these credentials belong to.
    pub api_url: String,
    pub device_id: String,
    pub storage_account_id: String,
    /// base64url P-256 private scalar of the signing (DPoP) key.
    pub sign_key: String,
    /// base64url P-256 private scalar of the HPKE key.
    pub enc_key: String,
    pub refresh_token: String,
    pub access_token: String,
    /// Unix seconds.
    pub access_expires_at: u64,
    /// The HPKE-sealed welcome from the approving browser.
    pub welcome: String,
    /// This computer's member in the account roster; the welcome is bound
    /// to it. Logins from before it was saved have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_id: Option<String>,
}

impl Credentials {
    pub const FORMAT: u32 = 1;

    pub fn load(home: &Home) -> Result<Option<Self>> {
        let path = home.credentials_file();
        let Some(bytes) = read_optional(&path)? else {
            return Ok(None);
        };
        let credentials: Self =
            serde_json::from_slice(&bytes).map_err(|error| CoreError::Invalid {
                path: path.clone(),
                message: error.to_string(),
            })?;
        if credentials.format != Self::FORMAT {
            return Err(CoreError::Invalid {
                path,
                message: format!("unsupported format {}", credentials.format),
            });
        }
        Ok(Some(credentials))
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        home.ensure_private_dir(home.root())?;
        home.ensure_private_dir(&home.state_dir())?;
        let path = home.credentials_file();
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| CoreError::Invalid {
            path: path.clone(),
            message: error.to_string(),
        })?;
        write_private(&path, &bytes)
    }

    /// Deletes the file; returns whether there was one.
    pub fn remove(home: &Home) -> Result<bool> {
        remove_optional(&home.credentials_file())
    }

    /// Whether the access token is still usable for at least `margin` seconds.
    pub fn access_fresh(&self, now: u64, margin: u64) -> bool {
        self.access_expires_at > now.saturating_add(margin)
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("api_url", &self.api_url)
            .field("device_id", &self.device_id)
            .field("storage_account_id", &self.storage_account_id)
            .field("access_expires_at", &self.access_expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Credentials {
        Credentials {
            format: Credentials::FORMAT,
            api_url: "http://localhost:5173".into(),
            device_id: "d".into(),
            storage_account_id: "s".into(),
            sign_key: "SIGN-SECRET".into(),
            enc_key: "ENC-SECRET".into(),
            refresh_token: "REFRESH-SECRET".into(),
            access_token: "ACCESS-SECRET".into(),
            access_expires_at: 1_000,
            welcome: "w".into(),
            member_id: Some("m".into()),
        }
    }

    #[test]
    fn saves_loads_and_removes() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("sp-creds-{}", std::process::id()));
        let home = Home::new(&dir);
        assert_eq!(Credentials::load(&home)?, None);
        sample().save(&home)?;
        assert_eq!(Credentials::load(&home)?, Some(sample()));
        assert!(Credentials::remove(&home)?);
        assert_eq!(Credentials::load(&home)?, None);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn debug_never_prints_secrets() {
        let text = format!("{:?}", sample());
        assert!(!text.contains("SECRET"), "{text}");
    }

    #[test]
    fn freshness_keeps_a_margin() {
        let credentials = sample();
        assert!(credentials.access_fresh(900, 60));
        assert!(!credentials.access_fresh(950, 60));
    }
}
