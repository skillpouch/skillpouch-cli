//! `skillpouch login`: create device keys, open a login request, let the user
//! approve it in a signed-in, unlocked browser, then redeem it for tokens.

use crate::browser;
use crate::context::{Context, VERSION, unix_now};
use crate::exit::{CANCELLED, Exit};
use anyhow::{Result, bail};
use sp_api::types::{CliAuthCreate, CliAuthToken, DeviceTokens};
use sp_core::home::user_home;
use sp_core::{Credentials, assistants};
use sp_crypto::confirm::confirm_code;
use sp_crypto::pkce::Pkce;
use sp_crypto::{EncKey, SignKey};
use std::process::ExitCode;
use std::time::Duration;

/// Seconds the server may hold each redeem call (the API allows up to 55).
const LONG_POLL: u64 = 25;
const NAME_MAX: usize = 64;

pub async fn run(
    ctx: &Context,
    token: Option<String>,
    name: Option<String>,
    no_browser: bool,
) -> Result<ExitCode> {
    if let Some(existing) = ctx.credentials()? {
        bail!(
            "This computer is already logged in to {}. Run `skillpouch logout` first.",
            existing.api_url
        );
    }
    if token
        .as_deref()
        .is_some_and(|token| !is_connect_token(token))
    {
        bail!("That doesn't look like a connect token (it starts with `sp_lt_`).");
    }

    let sign = SignKey::generate()?;
    let enc = EncKey::generate()?;
    let pkce = Pkce::generate()?;
    let sign_pub = sign.public_jwk()?;
    let enc_pub = enc.public_jwk()?;
    let via_token = token.is_some();
    let device_name = device_name(name.or_else(|| ctx.config.device_name.clone()));

    let request = ctx
        .client
        .create_login(
            &sign,
            &CliAuthCreate {
                code_challenge: pkce.challenge.clone(),
                device_name: device_name.clone(),
                assistants: user_home()
                    .map(|home| assistants::detect(&home))
                    .unwrap_or_default(),
                os: os(),
                arch: std::env::consts::ARCH.to_string(),
                cli_version: VERSION.to_string(),
                sign_pub: sign_pub.clone(),
                enc_pub: enc_pub.clone(),
                key_storage: "file",
                connect_token: token,
            },
        )
        .await?;
    let code = confirm_code(
        &request.request_id,
        &sign_pub,
        &enc_pub,
        &sign_pub.thumbprint(),
    );

    if via_token {
        println!("Approve this computer in the browser where you copied the command.");
    } else {
        println!(
            "Approve this computer in your browser:\n\n  {}\n",
            request.authorize_url
        );
        if !no_browser && browser::open(&request.authorize_url) {
            println!("(Opened it for you.)");
        }
    }
    println!("Confirmation code: {code}  — the browser must show the same code.");
    println!("Waiting for approval… (Ctrl+C to cancel)");

    let approved = tokio::select! {
        result = wait_for_approval(ctx, &sign, &request.request_id, &pkce.verifier, request.expires_in) => result?,
        _ = tokio::signal::ctrl_c() => {
            let _ = ctx.client.cancel_login(&sign, &request.request_id).await;
            return Err(Exit { code: CANCELLED, message: "Login cancelled.".into() }.into());
        }
    };
    let (tokens, welcome) = approved;

    Credentials {
        format: Credentials::FORMAT,
        api_url: ctx.client.base().to_string(),
        device_id: tokens.device_id,
        storage_account_id: tokens.storage_account_id,
        sign_key: sign.to_b64(),
        enc_key: enc.to_b64(),
        refresh_token: tokens.refresh_token,
        access_token: tokens.access_token,
        access_expires_at: unix_now() + tokens.expires_in,
        welcome,
    }
    .save(&ctx.home)?;

    println!("\nLogged in. This computer is \"{device_name}\" on your Devices page.");
    Ok(ExitCode::SUCCESS)
}

async fn wait_for_approval(
    ctx: &Context,
    key: &SignKey,
    request_id: &str,
    verifier: &str,
    expires_in: u64,
) -> Result<(DeviceTokens, String)> {
    let deadline = unix_now() + expires_in;
    loop {
        let started = unix_now();
        let answer = ctx
            .client
            .redeem_login(key, request_id, verifier, LONG_POLL)
            .await;
        match answer {
            Ok(CliAuthToken::Approved { tokens, welcome }) => return Ok((tokens, welcome)),
            Ok(CliAuthToken::Pending { interval }) => {
                // An early answer means the server didn't hold the call; pace ourselves.
                if unix_now().saturating_sub(started) < interval {
                    tokio::time::sleep(Duration::from_secs(interval.max(1))).await;
                }
            }
            Err(error) => {
                return Err(match error.code() {
                    Some("cli_auth_denied") => {
                        anyhow::anyhow!("The login was denied in the browser.")
                    }
                    Some("cli_auth_expired") => {
                        anyhow::anyhow!("The login request expired. Run `skillpouch login` again.")
                    }
                    _ => error.into(),
                });
            }
        }
        if unix_now() >= deadline {
            bail!("The login request expired. Run `skillpouch login` again.");
        }
    }
}

fn is_connect_token(token: &str) -> bool {
    token.strip_prefix("sp_lt_").is_some_and(|rest| {
        rest.len() == 32
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

/// The chosen name, else the hostname, trimmed to what the server accepts.
fn device_name(chosen: Option<String>) -> String {
    let name = chosen
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            gethostname::gethostname()
                .to_string_lossy()
                .trim()
                .to_string()
        });
    let name: String = name.chars().take(NAME_MAX).collect();
    if name.is_empty() {
        "My computer".into()
    } else {
        name
    }
}

fn os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_connect_tokens() {
        assert!(is_connect_token(&format!("sp_lt_{}", "a".repeat(32))));
        assert!(!is_connect_token(&format!("sp_lt_{}", "a".repeat(31))));
        assert!(!is_connect_token(&format!("sp_xx_{}", "a".repeat(32))));
        assert!(!is_connect_token(&format!("sp_lt_{}!", "a".repeat(31))));
    }

    #[test]
    fn device_names_fit_the_server() {
        assert_eq!(device_name(Some("  laptop ".into())), "laptop");
        assert_eq!(device_name(Some("x".repeat(100))).chars().count(), NAME_MAX);
        assert!(!device_name(Some("   ".into())).is_empty());
    }
}
