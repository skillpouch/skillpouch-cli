//! What a device needs to read the account's end-to-end encrypted metadata:
//! the welcome it got at login, the verified roster, its key envelopes and
//! the pouch names sealed in the roster.
//!
//! ```text
//! statement_hash = BLAKE3(canonical JSON bytes)
//! roster sig     = sign(signer, "skillpouch/v1/roster", bytes)
//! envelope       = HPKE_seal(member.enc_pub, "skillpouch/v1/key-envelope",
//!                            aad = pouch_id ‖ member_id ‖ u32_be(key_gen), PS_g ‖ PIS)
//! envelope sig   = sign(creator, "skillpouch/v1/key-envelope", aad ‖ enc ‖ ciphertext)
//! K_meta         = BLAKE3.derive_key("skillpouch/v1/meta", PS_g)
//! name_enc       = seal(K_meta, {name, description}, "skillpouch/v1/pouch-name" ‖ pouch_id)
//! ```

use crate::keys::{EncKey, PublicJwk};
use crate::{CryptoError, Result, b64, hpke, sealed};
use serde::Deserialize;
use serde_json::Value;
use sp_core::canonical::canonical_json;
use std::collections::{BTreeMap, HashMap};

const ROSTER: &str = "skillpouch/v1/roster";
const KEY_ENVELOPE: &str = "skillpouch/v1/key-envelope";
const WELCOME: &str = "skillpouch/v1/welcome";
const POUCH_NAME: &str = "skillpouch/v1/pouch-name";

/// A roster statement as the API returns it.
#[derive(Debug, Clone, Deserialize)]
pub struct RosterStatement {
    pub seq: u64,
    /// base64url of the signed canonical JSON.
    pub statement: String,
    pub statement_hash: String,
    pub signature: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyEnvelope {
    pub pouch_id: String,
    pub member_id: String,
    pub key_gen: u32,
    pub hpke_enc: String,
    pub ciphertext: String,
    pub created_by_member: String,
    pub signature: String,
}

/// Sealed to the device when it was approved: the roster it can trust.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Welcome {
    pub format: u8,
    pub account_id: String,
    pub genesis_hash: String,
    pub roster_head_seq: u64,
    pub roster_head_hash: String,
    pub member_id: String,
}

pub fn open_welcome(key: &EncKey, member_id: &str, sealed_b64: &str) -> Result<Welcome> {
    let bytes = b64::decode(sealed_b64).ok_or(CryptoError::Format("welcome is not base64url"))?;
    if bytes.len() < 65 + 16 {
        return Err(CryptoError::Format("welcome too short"));
    }
    let (enc, ciphertext) = bytes.split_at(65);
    let plaintext = hpke::open(
        key,
        enc,
        ciphertext,
        WELCOME.as_bytes(),
        member_id.as_bytes(),
    )?;
    let welcome: Welcome = serde_json::from_slice(&plaintext)
        .map_err(|_| CryptoError::Format("welcome is not JSON"))?;
    if welcome.format != 1 || welcome.member_id != member_id {
        return Err(CryptoError::Roster(
            "the welcome is not for this device".into(),
        ));
    }
    Ok(welcome)
}

#[derive(Debug, Clone)]
pub struct Member {
    pub kind: String,
    pub sign_pub: PublicJwk,
    pub added_seq: u64,
    pub removed_seq: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RosterPouch {
    pub name_enc: Option<String>,
    pub created_seq: u64,
    pub archived: bool,
}

/// The roster replayed from its genesis, every link and signature checked.
#[derive(Debug, Clone)]
pub struct Roster {
    pub head_seq: u64,
    pub head_hash: String,
    pub members: HashMap<String, Member>,
    pub pouches: HashMap<String, RosterPouch>,
}

#[derive(Deserialize)]
struct Body {
    format: u8,
    account_id: String,
    seq: u64,
    prev_hash: Option<String>,
    action: String,
    signer_member_id: String,
    #[serde(default)]
    members: Vec<MemberEntry>,
    member: Option<MemberEntry>,
    member_id: Option<String>,
    pouch: Option<PouchEntry>,
}

#[derive(Deserialize)]
struct MemberEntry {
    id: String,
    kind: String,
    sign_pub: PublicJwk,
}

#[derive(Deserialize)]
struct PouchEntry {
    id: String,
    name_enc: Option<String>,
}

fn fork(message: impl Into<String>) -> CryptoError {
    CryptoError::Roster(message.into())
}

/// Replays `statements` from genesis. `welcome` pins the genesis and the
/// head the device was shown at login: a server that forked or rolled the
/// roster back since is refused.
pub fn verify_roster(statements: &[RosterStatement], welcome: &Welcome) -> Result<Roster> {
    let mut sorted: Vec<&RosterStatement> = statements.iter().collect();
    sorted.sort_by_key(|statement| statement.seq);
    let mut roster: Option<Roster> = None;

    for wire in sorted {
        let bytes =
            b64::decode(&wire.statement).ok_or_else(|| fork("statement is not base64url"))?;
        let text = std::str::from_utf8(&bytes).map_err(|_| fork("statement is not UTF-8"))?;
        let value: Value = serde_json::from_str(text).map_err(|_| fork("statement is not JSON"))?;
        if canonical_json(&value) != text {
            return Err(fork(format!("roster #{} is not canonical", wire.seq)));
        }
        let body: Body = serde_json::from_value(value)
            .map_err(|_| fork(format!("roster #{} is malformed", wire.seq)))?;
        let hash = b64::encode(blake3::hash(&bytes).as_bytes());
        if hash != wire.statement_hash || body.seq != wire.seq || body.format != 1 {
            return Err(fork(format!("roster #{} hash mismatch", wire.seq)));
        }
        if body.account_id != welcome.account_id {
            return Err(fork("statement of another account"));
        }

        let state = match roster.as_mut() {
            None => {
                if body.seq != 0 || body.action != "genesis" || body.prev_hash.is_some() {
                    return Err(fork("roster does not start with genesis"));
                }
                if hash != welcome.genesis_hash {
                    return Err(fork("genesis differs from the one this computer trusts"));
                }
                let mut members = HashMap::new();
                for entry in body.members {
                    add_member(&mut members, entry, 0)?;
                }
                let signer = members
                    .get(&body.signer_member_id)
                    .ok_or_else(|| fork("genesis signer is not a member"))?;
                if !signer.sign_pub.verifies(&wire.signature, ROSTER, &bytes) {
                    return Err(CryptoError::Signature("genesis".into()));
                }
                roster.insert(Roster {
                    head_seq: 0,
                    head_hash: String::new(),
                    members,
                    pouches: HashMap::new(),
                })
            }
            Some(state) => {
                if body.seq != state.head_seq + 1
                    || body.prev_hash.as_deref() != Some(state.head_hash.as_str())
                {
                    return Err(fork(format!(
                        "roster #{} does not extend the chain",
                        body.seq
                    )));
                }
                let signer = state
                    .members
                    .get(&body.signer_member_id)
                    .filter(|m| {
                        m.added_seq < body.seq && m.removed_seq.is_none_or(|r| r >= body.seq)
                    })
                    .ok_or_else(|| {
                        CryptoError::Signature(format!("roster #{} signer", body.seq))
                    })?;
                if !signer.sign_pub.verifies(&wire.signature, ROSTER, &bytes) {
                    return Err(CryptoError::Signature(format!("roster #{}", body.seq)));
                }
                apply(state, body)?;
                state
            }
        };
        state.head_seq = wire.seq;
        state.head_hash = hash;
        if wire.seq == welcome.roster_head_seq && state.head_hash != welcome.roster_head_hash {
            return Err(fork("the roster changed after this computer saw it"));
        }
    }

    let roster = roster.ok_or_else(|| fork("the roster is empty"))?;
    if roster.head_seq < welcome.roster_head_seq {
        return Err(fork("the roster went backwards"));
    }
    Ok(roster)
}

fn add_member(members: &mut HashMap<String, Member>, entry: MemberEntry, seq: u64) -> Result<()> {
    if members.contains_key(&entry.id) {
        return Err(fork(format!("member {} added twice", entry.id)));
    }
    members.insert(
        entry.id,
        Member {
            kind: entry.kind,
            sign_pub: entry.sign_pub,
            added_seq: seq,
            removed_seq: None,
        },
    );
    Ok(())
}

fn apply(state: &mut Roster, body: Body) -> Result<()> {
    let seq = body.seq;
    match body.action.as_str() {
        "add" => add_member(
            &mut state.members,
            body.member.ok_or_else(|| fork("add without member"))?,
            seq,
        ),
        "remove" => {
            let member = body
                .member_id
                .and_then(|id| state.members.get_mut(&id))
                .filter(|m| m.removed_seq.is_none())
                .ok_or_else(|| fork("remove of an unknown or removed member"))?;
            member.removed_seq = Some(seq);
            Ok(())
        }
        "rotate_keys" => Ok(()),
        "create_pouch" => {
            let pouch = body
                .pouch
                .ok_or_else(|| fork("create_pouch without pouch"))?;
            if state.pouches.contains_key(&pouch.id) {
                return Err(fork("pouch created twice"));
            }
            state.pouches.insert(
                pouch.id,
                RosterPouch {
                    name_enc: pouch.name_enc,
                    created_seq: seq,
                    archived: false,
                },
            );
            Ok(())
        }
        "rename_pouch" | "archive_pouch" => {
            let entry = body.pouch.ok_or_else(|| fork("statement without pouch"))?;
            let pouch = state
                .pouches
                .get_mut(&entry.id)
                .ok_or_else(|| fork("statement for an unknown pouch"))?;
            if body.action == "rename_pouch" {
                if entry.name_enc.is_some() {
                    pouch.name_enc = entry.name_enc;
                }
            } else {
                pouch.archived = true;
            }
            Ok(())
        }
        "genesis" => Err(fork("a second genesis statement")),
        other => Err(fork(format!("unknown roster action {other}"))),
    }
}

/// `PS_g ‖ PIS` of every pouch and generation this device has an envelope
/// for. Envelopes not signed by a roster member are ignored.
pub fn open_envelopes(
    roster: &Roster,
    envelopes: &[KeyEnvelope],
    member_id: &str,
    key: &EncKey,
) -> HashMap<(String, u32), [u8; 64]> {
    let mut opened = HashMap::new();
    for envelope in envelopes.iter().filter(|e| e.member_id == member_id) {
        if let Ok(ps) = open_envelope(roster, envelope, key) {
            opened.insert((envelope.pouch_id.clone(), envelope.key_gen), ps);
        }
    }
    opened
}

fn open_envelope(roster: &Roster, envelope: &KeyEnvelope, key: &EncKey) -> Result<[u8; 64]> {
    let creator = roster
        .members
        .get(&envelope.created_by_member)
        .ok_or_else(|| CryptoError::Signature("key envelope from an unknown member".into()))?;
    let aad = [
        envelope.pouch_id.as_bytes(),
        envelope.member_id.as_bytes(),
        &envelope.key_gen.to_be_bytes(),
    ]
    .concat();
    let enc = b64::decode(&envelope.hpke_enc).ok_or(CryptoError::Format("bad envelope enc"))?;
    let ciphertext =
        b64::decode(&envelope.ciphertext).ok_or(CryptoError::Format("bad envelope ciphertext"))?;
    let signed = [&aad[..], &enc, &ciphertext].concat();
    if !creator
        .sign_pub
        .verifies(&envelope.signature, KEY_ENVELOPE, &signed)
    {
        return Err(CryptoError::Signature("key envelope".into()));
    }
    let material = hpke::open(key, &enc, &ciphertext, KEY_ENVELOPE.as_bytes(), &aad)?;
    material
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Format("bad key material length"))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PouchMeta {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// The names of the pouches this device can read.
pub fn pouch_names(
    roster: &Roster,
    keys: &HashMap<(String, u32), [u8; 64]>,
) -> BTreeMap<String, PouchMeta> {
    let mut names = BTreeMap::new();
    for (id, pouch) in &roster.pouches {
        let Some(sealed) = pouch.name_enc.as_deref().and_then(b64::decode) else {
            continue;
        };
        let Ok(key_gen) = sealed::key_gen(&sealed) else {
            continue;
        };
        let Some(material) = keys.get(&(id.clone(), key_gen)) else {
            continue;
        };
        let meta_key = blake3::derive_key("skillpouch/v1/meta", &material[..32]);
        let aad = [POUCH_NAME.as_bytes(), id.as_bytes()].concat();
        if let Some(meta) = sealed::open(&meta_key, &sealed, &aad)
            .ok()
            .and_then(|plain| serde_json::from_slice::<PouchMeta>(&plain).ok())
        {
            names.insert(id.clone(), meta);
        }
    }
    names
}
