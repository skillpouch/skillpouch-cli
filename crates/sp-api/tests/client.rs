use serde_json::{Value, json};
use sp_api::types::CliAuthToken;
use sp_api::{ApiError, Client};
use sp_crypto::SignKey;
use sp_crypto::{b64, dpop::access_token_hash};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const NONCE: &str = "7.server-nonce";

fn claims(request: &Request) -> Value {
    request
        .headers
        .get("DPoP")
        .and_then(|v| v.to_str().ok())
        .and_then(|jwt| jwt.split('.').nth(1).map(str::to_owned))
        .and_then(|part| b64::decode(&part))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "data": data, "meta": {} }))
}

fn error(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "ok": false,
        "error": { "code": code, "status": status, "message": "Nope.", "retryable": false },
        "meta": {}
    }))
}

/// Answers like the API: a proof without the current nonce gets `dpop_nonce_required`.
fn nonce_guard(data: Value) -> impl Fn(&Request) -> ResponseTemplate {
    move |request: &Request| {
        if claims(request)["nonce"] == NONCE {
            ok(data.clone())
        } else {
            error(401, "dpop_nonce_required").insert_header("DPoP-Nonce", NONCE)
        }
    }
}

fn client(server: &MockServer) -> Result<Client, ApiError> {
    Client::new(&server.uri(), "skillpouch-test")
}

#[tokio::test]
async fn fetches_a_nonce_then_reuses_it() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/cli-auth/token"))
        .respond_with(nonce_guard(json!({ "status": "pending", "interval": 2 })))
        .expect(3)
        .mount(&server)
        .await;
    let client = client(&server)?;
    let key = SignKey::generate()?;

    for _ in 0..2 {
        let token = client.redeem_login(&key, "req", "verifier", 0).await?;
        assert!(matches!(token, CliAuthToken::Pending { interval: 2 }));
    }

    let requests = server.received_requests().await.unwrap_or_default();
    let first = claims(&requests[0]);
    assert_eq!(first["htm"], "POST");
    assert_eq!(first["htu"], format!("{}/v1/cli-auth/token", server.uri()));
    assert!(first.get("nonce").is_none());
    assert_eq!(claims(&requests[2])["nonce"], NONCE);
    Ok(())
}

#[tokio::test]
async fn binds_the_access_token() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockServer::start().await;
    let me = json!({
        "user": { "id": "u", "email": "ada@example.com", "name": "Ada", "image": null, "role": "user" },
        "account": {
            "id": "a",
            "entitlements": { "plan_slug": "free", "plan_name": "Free", "source": "default", "pouch_limit": 1, "storage_bytes": 10 },
            "quota_bytes": 10, "used_bytes": 3, "roster_initialized": true,
            "genesis_hash": null, "roster_seq": 1, "delete_after": null
        },
        "session": null,
        "device": { "id": "d", "name": "laptop" }
    });
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(nonce_guard(me))
        .mount(&server)
        .await;
    let client = client(&server)?;
    let key = SignKey::generate()?;

    let me = client.me(&key, "access-123").await?;
    assert_eq!(me.user.email, "ada@example.com");
    assert_eq!(me.account.entitlements.plan_name, "Free");
    assert_eq!(me.device.map(|d| d.name).as_deref(), Some("laptop"));

    let requests = server.received_requests().await.unwrap_or_default();
    let last = requests.last().ok_or("no request")?;
    assert_eq!(
        last.headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok()),
        Some("DPoP access-123")
    );
    assert_eq!(claims(last)["ath"], access_token_hash("access-123"));
    Ok(())
}

#[tokio::test]
async fn reports_server_errors_by_code() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/cli-auth/token"))
        .respond_with(error(410, "cli_auth_expired"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/devices/token"))
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>Bad gateway</html>"))
        .mount(&server)
        .await;
    let client = client(&server)?;
    let key = SignKey::generate()?;

    let expired = client.redeem_login(&key, "req", "verifier", 0).await;
    assert!(
        matches!(expired, Err(ApiError::Server { ref code, status: 410, .. }) if code == "cli_auth_expired")
    );
    let gateway = client.refresh(&key, "refresh-token").await;
    assert!(matches!(
        gateway,
        Err(ApiError::Unexpected { status: 502, .. })
    ));
    Ok(())
}

#[tokio::test]
async fn parses_an_approved_login() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/cli-auth/token"))
        .respond_with(ok(json!({
            "status": "approved",
            "token_type": "DPoP",
            "access_token": "a",
            "expires_in": 900,
            "refresh_token": "r",
            "device_id": "d",
            "storage_account_id": "s",
            "welcome": "sealed"
        })))
        .mount(&server)
        .await;
    let client = client(&server)?;

    let token = client
        .redeem_login(&SignKey::generate()?, "req", "v", 30)
        .await?;
    let CliAuthToken::Approved { tokens, welcome } = token else {
        return Err("expected approved".into());
    };
    assert_eq!(
        (tokens.device_id.as_str(), welcome.as_str()),
        ("d", "sealed")
    );
    let body: Value = server.received_requests().await.unwrap_or_default()[0].body_json()?;
    assert_eq!(
        body,
        json!({ "request_id": "req", "code_verifier": "v", "wait": 30 })
    );
    Ok(())
}

#[tokio::test]
async fn logs_this_computer_out() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/devices/me/logout"))
        .respond_with(nonce_guard(
            json!({ "revoked": true, "rotation_pending": true }),
        ))
        .mount(&server)
        .await;
    let client = client(&server)?;
    let key = SignKey::generate()?;

    client.logout(&key, "access-123").await?;
    let requests = server.received_requests().await.unwrap_or_default();
    let last = requests.last().ok_or("no request")?;
    assert_eq!(claims(last)["htm"], "POST");
    assert_eq!(claims(last)["ath"], access_token_hash("access-123"));
    Ok(())
}
