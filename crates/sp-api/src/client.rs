use crate::types::{
    ArtifactList, CliAuthCreate, CliAuthCreated, CliAuthToken, DeviceList, DeviceTokens, Me,
    PouchList,
};
use crate::{ApiError, Result};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sp_crypto::SignKey;
use sp_crypto::dpop::{ProofInput, proof};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Client {
    http: reqwest::Client,
    base: String,
    /// The last `DPoP-Nonce` the server handed out.
    nonce: Mutex<Option<String>>,
}

struct Call<'a> {
    method: Method,
    path: &'a str,
    key: &'a SignKey,
    body: Option<Value>,
    access_token: Option<&'a str>,
    timeout: Duration,
}

impl Client {
    /// `base` is the server origin, e.g. `https://skillpouch.net`.
    pub fn new(base: &str, user_agent: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            nonce: Mutex::new(None),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Opens a login request; the proof must be signed by `body.sign_pub`'s key.
    pub async fn create_login(
        &self,
        key: &SignKey,
        body: &CliAuthCreate,
    ) -> Result<CliAuthCreated> {
        self.send(Call {
            body: Some(to_value(body)?),
            ..Call::new(Method::POST, "/v1/cli-auth/requests", key)
        })
        .await
    }

    /// Redeems an approved request; the server holds the call up to `wait` seconds.
    pub async fn redeem_login(
        &self,
        key: &SignKey,
        request_id: &str,
        code_verifier: &str,
        wait: u64,
    ) -> Result<CliAuthToken> {
        self.send(Call {
            body: Some(
                json!({ "request_id": request_id, "code_verifier": code_verifier, "wait": wait }),
            ),
            timeout: Duration::from_secs(wait) + TIMEOUT,
            ..Call::new(Method::POST, "/v1/cli-auth/token", key)
        })
        .await
    }

    pub async fn cancel_login(&self, key: &SignKey, request_id: &str) -> Result<()> {
        let path = format!("/v1/cli-auth/requests/{request_id}");
        self.send::<Value>(Call::new(Method::DELETE, &path, key))
            .await
            .map(drop)
    }

    /// Trades the (single-use) refresh token for new tokens.
    pub async fn refresh(&self, key: &SignKey, refresh_token: &str) -> Result<DeviceTokens> {
        self.send(Call {
            body: Some(json!({ "refresh_token": refresh_token })),
            ..Call::new(Method::POST, "/v1/devices/token", key)
        })
        .await
    }

    pub async fn me(&self, key: &SignKey, access_token: &str) -> Result<Me> {
        self.get("/v1/me", key, access_token).await
    }

    pub async fn pouches(&self, key: &SignKey, access_token: &str) -> Result<PouchList> {
        self.get("/v1/pouches", key, access_token).await
    }

    pub async fn devices(&self, key: &SignKey, access_token: &str) -> Result<DeviceList> {
        self.get("/v1/devices", key, access_token).await
    }

    /// Revokes this computer: the server forgets it and it leaves the devices list.
    pub async fn logout(&self, key: &SignKey, access_token: &str) -> Result<()> {
        self.send::<Value>(Call {
            access_token: Some(access_token),
            ..Call::new(Method::POST, "/v1/devices/me/logout", key)
        })
        .await
        .map(drop)
    }

    /// The first `limit` (at most 200) live items of a pouch.
    pub async fn artifacts(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        limit: u32,
    ) -> Result<ArtifactList> {
        let path = format!("/v1/pouches/{pouch_id}/artifacts?limit={limit}");
        self.get(&path, key, access_token).await
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        key: &SignKey,
        access_token: &str,
    ) -> Result<T> {
        self.send(Call {
            access_token: Some(access_token),
            ..Call::new(Method::GET, path, key)
        })
        .await
    }

    /// Sends with a fresh DPoP proof; retries once when the server asks for a nonce.
    async fn send<T: DeserializeOwned>(&self, call: Call<'_>) -> Result<T> {
        let mut retried = false;
        loop {
            match self.attempt(&call).await {
                Err(ApiError::Server { ref code, .. })
                    if code == "dpop_nonce_required" && !retried =>
                {
                    retried = true;
                }
                result => return result,
            }
        }
    }

    async fn attempt<T: DeserializeOwned>(&self, call: &Call<'_>) -> Result<T> {
        let url = format!("{}{}", self.base, call.path);
        let nonce = self.current_nonce();
        let dpop = proof(
            call.key,
            &ProofInput {
                method: call.method.as_str(),
                url: &url,
                issued_at: unix_now(),
                nonce: nonce.as_deref(),
                access_token: call.access_token,
            },
        )?;
        let mut request = self
            .http
            .request(call.method.clone(), &url)
            .timeout(call.timeout)
            .header("DPoP", dpop);
        if let Some(token) = call.access_token {
            request = request.header("Authorization", format!("DPoP {token}"));
        }
        if let Some(body) = &call.body {
            request = request.json(body);
        }
        let response = request.send().await?;
        if let Some(nonce) = response
            .headers()
            .get("DPoP-Nonce")
            .and_then(|v| v.to_str().ok())
        {
            self.remember_nonce(nonce);
        }
        let status = response.status();
        let text = response.text().await?;
        parse_envelope(status, &text)
    }

    fn current_nonce(&self) -> Option<String> {
        self.nonce.lock().map(|n| n.clone()).unwrap_or_default()
    }

    fn remember_nonce(&self, nonce: &str) {
        if let Ok(mut slot) = self.nonce.lock() {
            *slot = Some(nonce.to_string());
        }
    }
}

impl<'a> Call<'a> {
    fn new(method: Method, path: &'a str, key: &'a SignKey) -> Self {
        Self {
            method,
            path,
            key,
            body: None,
            access_token: None,
            timeout: TIMEOUT,
        }
    }
}

fn to_value(body: &impl Serialize) -> Result<Value> {
    serde_json::to_value(body).map_err(|error| ApiError::Unexpected {
        status: 0,
        message: error.to_string(),
    })
}

fn parse_envelope<T: DeserializeOwned>(status: StatusCode, text: &str) -> Result<T> {
    let unexpected = |message: String| ApiError::Unexpected {
        status: status.as_u16(),
        message,
    };
    let envelope: Value =
        serde_json::from_str(text).map_err(|_| unexpected(text.chars().take(200).collect()))?;
    if envelope["ok"] == true && status.is_success() {
        return serde_json::from_value(envelope["data"].clone())
            .map_err(|error| unexpected(error.to_string()));
    }
    let error = &envelope["error"];
    let Some(code) = error["code"].as_str() else {
        return Err(unexpected(text.chars().take(200).collect()));
    };
    Err(ApiError::Server {
        code: code.to_string(),
        status: status.as_u16(),
        message: error["message"].as_str().unwrap_or(code).to_string(),
        details: error.get("details").cloned(),
        retryable: error["retryable"] == true,
    })
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
