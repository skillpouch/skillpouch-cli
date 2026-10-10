//! Pouch names, decrypted here: the server only has them sealed. The roster
//! is checked against the welcome this computer got when it was approved, so
//! a server can't swap in pouches or keys of its own.

use crate::context::{Context, Session};
use sp_crypto::EncKey;
use sp_crypto::vault::{self, PouchMeta};
use std::collections::BTreeMap;

/// Said when the names can't be read; the dashboard shows pouch numbers.
pub type Missing = String;

pub async fn pouch_names(
    ctx: &Context,
    session: &Session,
) -> Result<BTreeMap<String, PouchMeta>, Missing> {
    let credentials = &session.credentials;
    let Some(member_id) = credentials.member_id.as_deref() else {
        return Err(
            "Pouch names show once this computer logs in again (Account → Log in again).".into(),
        );
    };
    let unreadable = |error: &dyn std::fmt::Display| format!("Couldn't read pouch names: {error}.");
    let key = EncKey::from_b64(&credentials.enc_key).map_err(|e| unreadable(&e))?;
    let welcome =
        vault::open_welcome(&key, member_id, &credentials.welcome).map_err(|e| unreadable(&e))?;
    let token = credentials.access_token.as_str();
    let (roster, envelopes) = tokio::try_join!(
        ctx.client.roster(&session.key, token),
        ctx.client.key_envelopes(&session.key, token),
    )
    .map_err(|e| unreadable(&e))?;
    let roster = vault::verify_roster(&roster.statements, &welcome).map_err(|e| unreadable(&e))?;
    let keys = vault::open_envelopes(&roster, &envelopes.key_envelopes, member_id, &key)
        .map_err(|e| unreadable(&e))?;
    Ok(vault::pouch_names(&roster, &keys))
}
