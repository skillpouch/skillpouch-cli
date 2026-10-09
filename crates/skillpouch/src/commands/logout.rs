use crate::context::{Context, Session};
use crate::ui;
use anyhow::Result;
use sp_core::Credentials;
use sp_crypto::SignKey;
use std::process::ExitCode;

pub async fn run(ctx: &Context) -> Result<ExitCode> {
    let Some(credentials) = Credentials::load(&ctx.home)? else {
        ui::hint("Not logged in, nothing to do.");
        return Ok(ExitCode::SUCCESS);
    };
    let spinner = ui::Spinner::start("Logging out…");
    let removed = forget(ctx, credentials.clone()).await;
    spinner.clear();
    removed?;
    ui::success("Logged out. This computer's keys are deleted and it's off your Devices page.");
    Ok(ExitCode::SUCCESS)
}

/// Removes this computer from the account, then deletes its keys and tokens.
/// The keys go even when the server can't be reached: logging out must work
/// offline. In that case the computer stays on the Devices page.
pub async fn forget(ctx: &Context, credentials: Credentials) -> Result<()> {
    let revoked = revoke(ctx, credentials.clone()).await;
    Credentials::remove(&ctx.home)?;
    // Nothing to sync without a login; the links keep working as they are.
    let _ = crate::daemon::stop(ctx);
    if !revoked {
        ui::warn(format!(
            "Couldn't reach the server. Remove this computer on your Devices page: {}",
            ui::accent(format!("{}/app/account/devices", credentials.api_url))
        ));
    }
    Ok(())
}

/// Tells the server these credentials' computer is gone. `false`: it may
/// still be listed (server unreachable, or the login was for another server).
pub async fn revoke(ctx: &Context, credentials: Credentials) -> bool {
    if credentials.api_url != ctx.client.base() {
        return false;
    }
    let Ok(key) = SignKey::from_b64(&credentials.sign_key) else {
        return false;
    };
    let mut session = Session { credentials, key };
    let Ok(token) = session.access_token(ctx, false).await else {
        // Already revoked elsewhere: nothing left to remove.
        return false;
    };
    match ctx.client.logout(&session.key, &token).await {
        Ok(()) => true,
        Err(error) => error.code() == Some("device_revoked"),
    }
}
