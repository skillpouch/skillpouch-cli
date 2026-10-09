//! The binary end to end, against an empty `SKILLPOUCH_HOME` and no server.

use assert_cmd::Command;
use tempfile::TempDir;

fn skillpouch(home: &TempDir) -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = Command::cargo_bin("skillpouch")?;
    command
        .env("SKILLPOUCH_HOME", home.path())
        // Nothing listens here; tests that reach the network would fail loudly.
        .env("SKILLPOUCH_API_URL", "http://127.0.0.1:9");
    Ok(command)
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn prints_its_version() -> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    let output = skillpouch(&home)?.arg("--version").output()?;
    assert!(output.status.success());
    assert_eq!(
        stdout(&output).trim(),
        format!("skillpouch {}", env!("CARGO_PKG_VERSION"))
    );
    Ok(())
}

#[test]
fn status_without_login_exits_3() -> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    let output = skillpouch(&home)?.arg("status").output()?;
    assert_eq!(output.status.code(), Some(3));
    assert!(
        stderr(&output).contains("skillpouch login"),
        "{}",
        stderr(&output)
    );
    Ok(())
}

#[test]
fn logout_without_login_is_fine() -> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    let output = skillpouch(&home)?.arg("logout").output()?;
    assert!(output.status.success());
    assert!(stdout(&output).contains("Not logged in"));
    Ok(())
}

#[test]
fn login_rejects_a_malformed_token_before_any_request() -> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    let output = skillpouch(&home)?.args(["login", "nope"]).output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("connect token"),
        "{}",
        stderr(&output)
    );
    Ok(())
}

#[test]
fn unknown_commands_are_usage_errors() -> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    let output = skillpouch(&home)?.arg("frobnicate").output()?;
    assert_eq!(output.status.code(), Some(2));
    Ok(())
}

#[test]
fn a_broken_config_is_reported() -> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    std::fs::write(home.path().join("config.toml"), "api_url = [")?;
    let output = skillpouch(&home)?.arg("status").output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("config.toml"),
        "{}",
        stderr(&output)
    );
    Ok(())
}

#[test]
fn logging_in_again_replaces_the_login_instead_of_refusing()
-> Result<(), Box<dyn std::error::Error>> {
    let home = TempDir::new()?;
    std::fs::create_dir_all(home.path().join("state"))?;
    let credentials = serde_json::json!({
        "format": 1, "api_url": "http://127.0.0.1:9", "device_id": "d", "storage_account_id": "s",
        "sign_key": "k", "enc_key": "k", "refresh_token": "r", "access_token": "a",
        "access_expires_at": 0, "welcome": "w",
    });
    let file = home.path().join("state/credentials.json");
    std::fs::write(&file, credentials.to_string())?;
    let token = format!("sp_lt_{}", "a".repeat(32));
    let output = skillpouch(&home)?.args(["login", &token]).output()?;
    assert!(
        stdout(&output).contains("Replaces the login"),
        "{}",
        stdout(&output)
    );
    assert!(!stderr(&output).contains("logout"), "{}", stderr(&output));
    // Nothing listens on the server, so the login fails and the old one stays.
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&file)?, credentials.to_string());
    Ok(())
}
