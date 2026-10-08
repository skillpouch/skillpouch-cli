//! `skillpouch login`: create device keys, open a login request, let the user
//! approve it in a signed-in, unlocked browser, then redeem it for tokens.

use crate::browser;
use crate::context::{Context, VERSION, unix_now};
use crate::exit::{CANCELLED, Exit};
use crate::ui::{self, Card, Spinner};
use anyhow::Result;
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
    if token
        .as_deref()
        .is_some_and(|token| !is_connect_token(token))
    {
        return Err(Exit::failure(
            "That doesn't look like a connect token.",
            "Connect tokens start with sp_lt_. Copy the whole command from the web app.",
        ));
    }

    ui::banner(&format!("v{VERSION}"));
    // Logging in again replaces the current login, but only once the new one
    // is approved: a cancelled or denied login leaves this computer as it was.
    let replacing = Credentials::load(&ctx.home).ok().flatten();
    if let Some(existing) = &replacing {
        ui::hint(format!(
            "Already logged in to {}. Approving this login replaces it with new keys.",
            existing.api_url
        ));
        println!();
    }
    let via_token = token.is_some();
    let spinner = Spinner::start("Creating this computer's keys…");
    let pending = begin(ctx, token, name).await;
    spinner.clear();
    let pending = pending?;

    println!(
        "{} {}\n",
        ui::heading("Connecting"),
        ui::accent(&pending.device_name)
    );
    if via_token {
        step(1, "Approve it in the browser where you copied the command");
    } else {
        step(1, "Approve this computer in your browser");
        println!("     {}", ui::accent(&pending.authorize_url));
        if !no_browser && browser::open(&pending.authorize_url) {
            println!("     {}", ui::dim("Opened it for you."));
        }
    }
    println!();
    step(2, "Check that the browser shows this code");
    ui::code_box(&pending.code);
    println!();

    let spinner = Spinner::with_elapsed(format!(
        "Waiting for approval {}",
        ui::dim("· Ctrl+C to cancel")
    ));
    let approved = tokio::select! {
        result = pending.wait(ctx) => {
            spinner.clear();
            result?
        }
        _ = tokio::signal::ctrl_c() => {
            spinner.clear();
            pending.cancel(ctx).await;
            return Err(Exit::with_code(CANCELLED, "Login cancelled.", Some("Nothing was saved. Run `skillpouch login` to try again.")));
        }
    };
    pending.save(ctx, approved)?;

    ui::success(if replacing.is_some() {
        "Logged in with new keys"
    } else {
        "Logged in"
    });
    println!();
    let card = Card::new("This computer")
        .row("Name", pending.device_name.as_str())
        .row("Server", ctx.client.base())
        .row("Keys", ui::dim(ctx.home.root().display()).to_string());
    println!("{}", card.render(ui::width()));
    println!();
    if replacing.is_some() {
        ui::hint("The old login still shows on your Devices page; remove it there.");
    }
    ui::hint("Run `skillpouch status` to see your account.");
    Ok(ExitCode::SUCCESS)
}

/// A login request waiting for the user to approve it in a browser.
pub struct Pending {
    sign: SignKey,
    enc: EncKey,
    verifier: String,
    request_id: String,
    expires_in: u64,
    pub device_name: String,
    pub authorize_url: String,
    /// What the browser must show, e.g. `2605-3825`.
    pub code: String,
}

/// Creates this computer's keys and opens a login request.
pub async fn begin(ctx: &Context, token: Option<String>, name: Option<String>) -> Result<Pending> {
    let sign = SignKey::generate()?;
    let enc = EncKey::generate()?;
    let pkce = Pkce::generate()?;
    let sign_pub = sign.public_jwk()?;
    let enc_pub = enc.public_jwk()?;
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
    Ok(Pending {
        sign,
        enc,
        verifier: pkce.verifier,
        request_id: request.request_id,
        expires_in: request.expires_in,
        device_name,
        authorize_url: request.authorize_url,
        code,
    })
}

impl Pending {
    /// Waits until the login is approved, denied or expires.
    pub async fn wait(&self, ctx: &Context) -> Result<(DeviceTokens, String)> {
        wait_for_approval(
            ctx,
            &self.sign,
            &self.request_id,
            &self.verifier,
            self.expires_in,
        )
        .await
    }

    /// Tells the server the user gave up; failing to is harmless (it expires).
    pub async fn cancel(&self, ctx: &Context) {
        let _ = ctx.client.cancel_login(&self.sign, &self.request_id).await;
    }

    /// Saves the approved login, replacing any earlier one.
    pub fn save(&self, ctx: &Context, (tokens, welcome): (DeviceTokens, String)) -> Result<()> {
        Credentials {
            format: Credentials::FORMAT,
            api_url: ctx.client.base().to_string(),
            device_id: tokens.device_id,
            storage_account_id: tokens.storage_account_id,
            sign_key: self.sign.to_b64(),
            enc_key: self.enc.to_b64(),
            refresh_token: tokens.refresh_token,
            access_token: tokens.access_token,
            access_expires_at: unix_now() + tokens.expires_in,
            welcome,
        }
        .save(&ctx.home)?;
        Ok(())
    }
}

fn step(number: u8, text: &str) {
    println!("  {}  {}", ui::accent(number).bold(), text);
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
                    Some("cli_auth_denied") => Exit::failure(
                        "The login was denied in the browser.",
                        "If that wasn't you, nothing else is needed. Otherwise run `skillpouch login` again.",
                    ),
                    Some("cli_auth_expired") => expired(),
                    _ => error.into(),
                });
            }
        }
        if unix_now() >= deadline {
            return Err(expired());
        }
    }
}

fn expired() -> anyhow::Error {
    Exit::failure(
        "The login request expired.",
        "Run `skillpouch login` again.",
    )
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
