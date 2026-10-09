use crate::types::{
    ArtifactList, ArtifactRecord, BlobCheck, BlobMissing, CliAuthCreate, CliAuthCreated,
    CliAuthToken, CommitRequest, CommitResponse, ConflictList, DeviceList, DeviceTokens, EventPage,
    KeyEnvelopes, Me, PouchList, Roster, UsageReport, UsageSummary,
};
use crate::{ApiError, Result};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sp_core::canonical::canonical_json;
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
    /// Sent as `application/octet-stream` instead of `body`.
    bytes: Option<Vec<u8>>,
    headers: Vec<(&'static str, String)>,
    access_token: Option<&'a str>,
    timeout: Duration,
}

/// A successful response before its envelope is read.
struct Reply {
    status: StatusCode,
    bytes: Vec<u8>,
    content_type: String,
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

    /// Every roster statement, from genesis.
    pub async fn roster(&self, key: &SignKey, access_token: &str) -> Result<Roster> {
        self.get("/v1/account/roster?after=-1", key, access_token)
            .await
    }

    /// The key envelopes sealed to this device, every pouch and generation.
    pub async fn key_envelopes(&self, key: &SignKey, access_token: &str) -> Result<KeyEnvelopes> {
        self.get("/v1/account/key-envelopes", key, access_token)
            .await
    }

    /// The pouch this computer syncs (`None`: none).
    pub async fn set_active_pouch(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: Option<&str>,
    ) -> Result<()> {
        self.send::<Value>(Call {
            access_token: Some(access_token),
            body: Some(json!({ "pouch_id": pouch_id })),
            ..Call::new(Method::PUT, "/v1/devices/me/active-pouch", key)
        })
        .await
        .map(drop)
    }

    /// Reports this computer's token counts (at most 2000 rows per call).
    pub async fn report_usage(
        &self,
        key: &SignKey,
        access_token: &str,
        source_id: &str,
        report: &UsageReport,
    ) -> Result<()> {
        let path = format!("/v1/usage/sources/{source_id}");
        self.send::<Value>(Call {
            access_token: Some(access_token),
            body: Some(to_value(report)?),
            ..Call::new(Method::PUT, &path, key)
        })
        .await
        .map(drop)
    }

    /// The account's token usage, summed over its computers.
    pub async fn usage(&self, key: &SignKey, access_token: &str) -> Result<UsageSummary> {
        self.get("/v1/usage", key, access_token).await
    }

    /// One page of a pouch's items with their head revisions, deleted ones
    /// included; the cursor of the next page, if any.
    pub async fn artifact_page(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<ArtifactRecord>, Option<String>)> {
        #[derive(serde::Deserialize)]
        struct Page {
            items: Vec<ArtifactRecord>,
        }
        let mut path = format!("/v1/pouches/{pouch_id}/artifacts?limit=200&include_deleted=true");
        if let Some(cursor) = cursor {
            path.push_str("&cursor=");
            path.push_str(&encode_query(cursor));
        }
        let reply = self
            .exchange(&Call {
                access_token: Some(access_token),
                ..Call::new(Method::GET, &path, key)
            })
            .await?;
        let (page, next) =
            parse_envelope::<Page>(reply.status, &String::from_utf8_lossy(&reply.bytes))?;
        Ok((page.items, next))
    }

    /// Which of these blobs the server doesn't have yet (and reserves quota for them).
    pub async fn check_blobs(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        blobs: &[BlobCheck],
    ) -> Result<BlobMissing> {
        let path = format!("/v1/pouches/{pouch_id}/blobs/check");
        self.send(Call {
            access_token: Some(access_token),
            body: Some(json!({ "blobs": blobs })),
            ..Call::new(Method::POST, &path, key)
        })
        .await
    }

    /// Uploads framed blobs (`frame_batch`).
    pub async fn upload_blobs(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        framed: Vec<u8>,
    ) -> Result<()> {
        let path = format!("/v1/pouches/{pouch_id}/blobs/batch");
        self.send::<Value>(Call {
            access_token: Some(access_token),
            bytes: Some(framed),
            timeout: Duration::from_secs(120),
            ..Call::new(Method::POST, &path, key)
        })
        .await
        .map(drop)
    }

    /// Downloads blobs as one framed body (`parse_fetched`).
    pub async fn fetch_blobs(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        ids: &[String],
    ) -> Result<Vec<u8>> {
        let path = format!("/v1/pouches/{pouch_id}/blobs/fetch");
        let reply = self
            .exchange(&Call {
                access_token: Some(access_token),
                body: Some(json!({ "ids": ids })),
                timeout: Duration::from_secs(120),
                ..Call::new(Method::POST, &path, key)
            })
            .await?;
        if !reply.content_type.starts_with("application/octet-stream") {
            return Err(ApiError::Unexpected {
                status: reply.status.as_u16(),
                message: format!("expected blob frames, got {}", reply.content_type),
            });
        }
        Ok(reply.bytes)
    }

    /// Commits signed operations. The device also signs the request itself:
    /// a detached JWS over BLAKE3 of the canonical body.
    pub async fn commit(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        request: &CommitRequest,
    ) -> Result<CommitResponse> {
        let body = to_value(request)?;
        let digest = blake3::hash(canonical_json(&body).as_bytes());
        let path = format!("/v1/pouches/{pouch_id}/operations");
        self.send(Call {
            access_token: Some(access_token),
            body: Some(body),
            headers: vec![("Skillpouch-Signature", key.detached_jws(digest.as_bytes()))],
            timeout: Duration::from_secs(120),
            ..Call::new(Method::POST, &path, key)
        })
        .await
    }

    /// Events after `after`; with `wait` the server holds the call until
    /// something happens or the wait runs out.
    pub async fn events(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        epoch: &str,
        after: u64,
        wait: u64,
    ) -> Result<EventPage> {
        let path = format!("/v1/pouches/{pouch_id}/events?after={after}&epoch={epoch}&wait={wait}");
        self.send(Call {
            access_token: Some(access_token),
            timeout: Duration::from_secs(wait) + TIMEOUT,
            ..Call::new(Method::GET, &path, key)
        })
        .await
    }

    /// The pouch's open conflicts.
    pub async fn open_conflicts(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
    ) -> Result<ConflictList> {
        let path = format!("/v1/pouches/{pouch_id}/conflicts?state=open");
        self.get(&path, key, access_token).await
    }

    /// Tells the server (and the web's device list) how far this computer synced.
    pub async fn report_cursor(
        &self,
        key: &SignKey,
        access_token: &str,
        pouch_id: &str,
        epoch: &str,
        acked_seq: u64,
    ) -> Result<()> {
        self.send::<Value>(Call {
            access_token: Some(access_token),
            body: Some(json!({ "pouch_id": pouch_id, "epoch": epoch, "acked_seq": acked_seq })),
            ..Call::new(Method::PUT, "/v1/devices/me/cursor", key)
        })
        .await
        .map(drop)
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

    async fn send<T: DeserializeOwned>(&self, call: Call<'_>) -> Result<T> {
        let reply = self.exchange(&call).await?;
        parse_envelope(reply.status, &String::from_utf8_lossy(&reply.bytes)).map(|(data, _)| data)
    }

    /// Sends with a fresh DPoP proof; retries once when the server asks for a
    /// nonce. Error envelopes become `ApiError::Server`.
    async fn exchange(&self, call: &Call<'_>) -> Result<Reply> {
        let mut retried = false;
        loop {
            let reply = self.attempt(call).await?;
            if reply.status.is_success() {
                return Ok(reply);
            }
            match parse_envelope::<Value>(reply.status, &String::from_utf8_lossy(&reply.bytes)) {
                Err(ApiError::Server { ref code, .. })
                    if code == "dpop_nonce_required" && !retried =>
                {
                    retried = true;
                }
                Err(error) => return Err(error),
                Ok(_) => {
                    return Err(ApiError::Unexpected {
                        status: reply.status.as_u16(),
                        message: "error status with a success envelope".into(),
                    });
                }
            }
        }
    }

    async fn attempt(&self, call: &Call<'_>) -> Result<Reply> {
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
        for (name, value) in &call.headers {
            request = request.header(*name, value);
        }
        if let Some(bytes) = &call.bytes {
            request = request
                .header("Content-Type", "application/octet-stream")
                .body(bytes.clone());
        } else if let Some(body) = &call.body {
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
        let content_type = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = response.bytes().await?.to_vec();
        Ok(Reply {
            status,
            bytes,
            content_type,
        })
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
            bytes: None,
            headers: Vec::new(),
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

/// The envelope's data and, for lists, `meta.next_cursor`.
fn parse_envelope<T: DeserializeOwned>(
    status: StatusCode,
    text: &str,
) -> Result<(T, Option<String>)> {
    let unexpected = |message: String| ApiError::Unexpected {
        status: status.as_u16(),
        message,
    };
    let envelope: Value =
        serde_json::from_str(text).map_err(|_| unexpected(text.chars().take(200).collect()))?;
    if envelope["ok"] == true && status.is_success() {
        let next = envelope["meta"]["next_cursor"].as_str().map(str::to_string);
        return serde_json::from_value(envelope["data"].clone())
            .map(|data| (data, next))
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

/// Percent-encodes a query value (cursors are opaque strings).
fn encode_query(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
