//! What every command needs: the home folder, settings, the API client and,
//! once logged in, this computer's credentials with a usable access token.

use crate::exit::Exit;
use anyhow::{Context as _, Result};
use sp_api::{ApiError, Client};
use sp_core::{Config, Credentials, Home};
use sp_crypto::SignKey;
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Refresh the access token when it has less than this many seconds left.
const REFRESH_MARGIN: u64 = 60;

pub struct Context {
    pub home: Home,
    pub config: Config,
    pub client: Client,
}

impl Context {
    pub fn load() -> Result<Self> {
        let home = Home::locate()?;
        let config = Config::load(&home)?;
        let client = Client::new(&config.api_url(), &format!("skillpouch-cli/{VERSION}"))?;
        Ok(Self {
            home,
            config,
            client,
        })
    }

    /// The saved credentials for this server, if any.
    pub fn credentials(&self) -> Result<Option<Credentials>> {
        let credentials = Credentials::load(&self.home)?;
        Ok(credentials.filter(|c| c.api_url == self.client.base()))
    }

    pub fn session(&self) -> Result<Session> {
        let credentials = self.credentials()?.ok_or_else(|| {
            Exit::not_logged_in(format!(
                "Not logged in to {}. Run `skillpouch login`.",
                self.client.base()
            ))
        })?;
        let key = SignKey::from_b64(&credentials.sign_key).context(
            "the saved device key is damaged; run `skillpouch logout`, then log in again",
        )?;
        Ok(Session { credentials, key })
    }
}

pub struct Session {
    pub credentials: Credentials,
    pub key: SignKey,
}

impl Session {
    /// A valid access token, refreshing (and saving) it when needed.
    pub async fn access_token(&mut self, ctx: &Context, force_refresh: bool) -> Result<String> {
        if !force_refresh && self.credentials.access_fresh(unix_now(), REFRESH_MARGIN) {
            return Ok(self.credentials.access_token.clone());
        }
        let tokens = ctx
            .client
            .refresh(&self.key, &self.credentials.refresh_token)
            .await
            .map_err(signed_out)?;
        self.credentials.access_token = tokens.access_token;
        self.credentials.refresh_token = tokens.refresh_token;
        self.credentials.access_expires_at = unix_now() + tokens.expires_in;
        self.credentials.save(&ctx.home)?;
        Ok(self.credentials.access_token.clone())
    }
}

/// Turns "this device can't sign in any more" into exit code 3.
pub fn signed_out(error: ApiError) -> anyhow::Error {
    match error.code() {
        Some("device_revoked") => Exit::not_logged_in(
            "This computer was removed from your account. Run `skillpouch logout`, then `skillpouch login`.",
        ),
        Some("refresh_invalid" | "refresh_reused" | "member_inactive") => Exit::not_logged_in(
            "This computer's sign-in is no longer valid. Run `skillpouch logout`, then `skillpouch login`.",
        ),
        _ => error.into(),
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
