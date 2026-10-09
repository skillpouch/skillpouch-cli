//! Cross-checks pouch sealing with `@skillpouch/crypto`:
//! `seal` prints a revision and a blob sealed here; `open <file>` opens
//! ones sealed by the TypeScript side.

use sp_crypto::pouch::{ManifestEntry, PouchKeys, RevisionBody, RevisionColumns};
use sp_crypto::{PublicJwk, SignKey, b64};

fn keys() -> Result<PouchKeys, Box<dyn std::error::Error>> {
    let mut material = [7u8; 64];
    material[32..].fill(9);
    Ok(PouchKeys::new(
        "0199aaaa-0000-7000-8000-000000000001",
        &[(1, material)],
    )?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let keys = keys()?;
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("open") {
        let text = std::fs::read_to_string(args.get(2).ok_or("file")?)?;
        let v: serde_json::Value = serde_json::from_str(&text)?;
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        let jwk: PublicJwk = serde_json::from_value(v["sign_pub"].clone())?;
        let body = keys.open_revision(
            &RevisionColumns {
                artifact_id: &s("artifact_id"),
                revision_id: &s("revision_id"),
                parent_id: None,
                envelope: &s("envelope"),
                signature: &s("signature"),
                signer_member_id: &s("member_id"),
            },
            &jwk,
        )?;
        let blob = keys.open_blob(&s("blob_id"), &b64::decode(&s("blob")).ok_or("blob")?)?;
        println!(
            "opened {} {} → {}",
            body.kind,
            body.logical_key,
            String::from_utf8(blob)?
        );
        println!(
            "name_index equal: {}",
            keys.name_index("skill", "global", "PDF-Tools") == s("name_index")
        );
        return Ok(());
    }
    let author = SignKey::generate()?;
    let blob = keys.seal_blob("# PDF from Rust\n".as_bytes())?;
    let body = RevisionBody {
        format: 1,
        artifact_id: "0199aaaa-0000-7000-8000-0000000000a1".into(),
        revision_id: "0199aaaa-0000-7000-8000-0000000000b1".into(),
        parent_id: None,
        merge_parent_id: None,
        kind: "skill".into(),
        scope: "global".into(),
        logical_key: "pdf-tools".into(),
        deleted: false,
        entries: vec![ManifestEntry {
            path: "SKILL.md".into(),
            blob: blob.id.clone(),
            size: blob.size,
            mode: "file".into(),
        }],
        roster_head: "h".into(),
        author_member_id: "0199aaaa-0000-7000-8000-0000000000c1".into(),
        created_at: "2026-10-09T00:00:00.000Z".into(),
    };
    let sealed = keys.seal_revision(&author, &body)?;
    println!(
        "{}",
        serde_json::json!({
            "sign_pub": author.public_jwk()?,
            "envelope": sealed.envelope,
            "signature": sealed.signature,
            "blob_id": blob.id,
            "blob": b64::encode(&blob.bytes),
            "name_index": keys.name_index("skill", "global", "PDF-Tools"),
        })
    );
    Ok(())
}
