//! Reads an account the web's crypto code built (`tests/fixtures/account.json`):
//! the welcome, the roster, key envelopes across a key rotation, and the
//! pouch names.

use serde::Deserialize;
use sp_crypto::EncKey;
use sp_crypto::vault::{
    KeyEnvelope, RosterStatement, Welcome, open_envelopes, open_welcome, pouch_names, verify_roster,
};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    account_id: String,
    member_id: String,
    enc_key: String,
    welcome: String,
    roster: Vec<RosterStatement>,
    key_envelopes: Vec<KeyEnvelope>,
    names: BTreeMap<String, String>,
}

fn fixture() -> Fixture {
    let text = include_str!("fixtures/account.json");
    serde_json::from_str(text).unwrap_or_else(|e| panic!("bad fixture: {e}"))
}

fn welcome(fixture: &Fixture) -> (EncKey, Welcome) {
    let key = EncKey::from_b64(&fixture.enc_key).unwrap_or_else(|e| panic!("{e}"));
    let welcome = open_welcome(&key, &fixture.member_id, &fixture.welcome)
        .unwrap_or_else(|e| panic!("welcome: {e}"));
    (key, welcome)
}

#[test]
fn reads_the_pouch_names_the_web_sealed() {
    let fixture = fixture();
    let (key, welcome) = welcome(&fixture);
    assert_eq!(welcome.account_id, fixture.account_id);

    let roster = verify_roster(&fixture.roster, &welcome).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(roster.head_seq, 7);
    let keys = open_envelopes(&roster, &fixture.key_envelopes, &fixture.member_id, &key)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(keys.len(), 4);

    let names: BTreeMap<String, String> = pouch_names(&roster, &keys)
        .into_iter()
        .map(|(id, meta)| (id, meta.name))
        .collect();
    assert_eq!(names, fixture.names);
}

#[test]
fn refuses_a_roster_the_server_changed() {
    let fixture = fixture();
    let (_, welcome) = welcome(&fixture);

    let mut cut = fixture.roster.clone();
    cut.truncate(2);
    assert!(verify_roster(&cut, &welcome).is_err(), "rolled back");

    let mut forged = fixture.roster.clone();
    forged[4].signature = forged[3].signature.clone();
    assert!(verify_roster(&forged, &welcome).is_err(), "bad signature");

    let mut other = welcome.clone();
    other.genesis_hash = "AAAA".into();
    assert!(
        verify_roster(&fixture.roster, &other).is_err(),
        "another genesis"
    );
}

#[test]
fn ignores_envelopes_nobody_in_the_roster_signed() {
    let fixture = fixture();
    let (key, welcome) = welcome(&fixture);
    let roster = verify_roster(&fixture.roster, &welcome).unwrap_or_else(|e| panic!("{e}"));
    let mut envelopes = fixture.key_envelopes.clone();
    for envelope in &mut envelopes {
        envelope.signature = fixture.key_envelopes[0].signature.clone();
    }
    // Only the first still carries its own signature.
    assert_eq!(
        open_envelopes(&roster, &envelopes, &fixture.member_id, &key)
            .map(|keys| keys.len())
            .unwrap_or_default(),
        1
    );
}

#[test]
fn a_welcome_for_another_device_does_not_open() {
    let fixture = fixture();
    let key = EncKey::from_b64(&fixture.enc_key).unwrap_or_else(|e| panic!("{e}"));
    assert!(open_welcome(&key, "someone-else", &fixture.welcome).is_err());
}
