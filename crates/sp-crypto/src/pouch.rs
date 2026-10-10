//! A pouch's keys and what they seal: file contents (blobs) and revision
//! envelopes. Byte-compatible with `@skillpouch/crypto`.
//!
//! ```text
//! K_content = BLAKE3.derive_key("skillpouch/v1/content", PS_g)
//! K_meta    = BLAKE3.derive_key("skillpouch/v1/meta", PS_g)
//! K_blob_id = BLAKE3.derive_key("skillpouch/v1/blob-id", PIS)
//! K_name    = BLAKE3.derive_key("skillpouch/v1/name-index", PIS)
//!
//! blob_id    = BLAKE3.keyed_hash(K_blob_id, plaintext)
//! blob       = seal(K_content, plaintext, "skillpouch/v1/blob" ‖ pouch_id ‖ blob_id)
//! name_index = BLAKE3.keyed_hash(K_name, "v1" ‖ kind ‖ 0 ‖ scope ‖ 0 ‖ lowercase(name))
//! envelope   = seal(K_meta, canonical JSON body,
//!                   "skillpouch/v1/revision" ‖ pouch_id ‖ artifact_id ‖ revision_id)
//! signature  = sign(author, "skillpouch/v1/revision-sig",
//!                   pouch_id ‖ revision_id ‖ BLAKE3(envelope))
//! ```

use crate::keys::{PublicJwk, SignKey};
use crate::{CryptoError, Result, b64, sealed};
use serde::{Deserialize, Serialize};
use sp_core::canonical::canonical_json;
use std::collections::BTreeMap;

const REVISION_SIG: &str = "skillpouch/v1/revision-sig";

#[derive(Clone)]
struct Generation {
    content: [u8; 32],
    meta: [u8; 32],
}

/// Every key generation of one pouch this device holds.
#[derive(Clone)]
pub struct PouchKeys {
    pub pouch_id: String,
    current: u32,
    generations: BTreeMap<u32, Generation>,
    blob_id_key: [u8; 32],
    name_key: [u8; 32],
}

impl std::fmt::Debug for PouchKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PouchKeys")
            .field("pouch_id", &self.pouch_id)
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

/// Sealed file contents, ready for `blobs/batch`.
#[derive(Debug, Clone)]
pub struct SealedBlob {
    pub id: String,
    pub bytes: Vec<u8>,
    /// base64url BLAKE3 of `bytes`.
    pub hash: String,
    /// Plaintext size.
    pub size: u64,
}

/// One file of a revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub blob: String,
    pub size: u64,
    pub mode: String,
}

/// What the server must not see about a version of an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionBody {
    pub format: u8,
    pub artifact_id: String,
    pub revision_id: String,
    pub parent_id: Option<String>,
    pub merge_parent_id: Option<String>,
    pub kind: String,
    pub scope: String,
    pub logical_key: String,
    pub deleted: bool,
    pub entries: Vec<ManifestEntry>,
    pub roster_head: String,
    pub author_member_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct SealedRevision {
    pub envelope: String,
    pub signature: String,
    pub key_gen: u32,
}

/// The server columns a decrypted body must agree with.
pub struct RevisionColumns<'a> {
    pub artifact_id: &'a str,
    pub revision_id: &'a str,
    pub parent_id: Option<&'a str>,
    pub envelope: &'a str,
    pub signature: &'a str,
    pub signer_member_id: &'a str,
}

impl PouchKeys {
    /// From the opened key envelopes (`PS_g ‖ PIS` per generation, ≥ 1).
    pub fn new(pouch_id: &str, opened: &[(u32, [u8; 64])]) -> Result<Self> {
        let Some((_, first)) = opened.first() else {
            return Err(CryptoError::Format("no key envelopes for this pouch"));
        };
        let generations: BTreeMap<u32, Generation> = opened
            .iter()
            .map(|(key_gen, material)| {
                let ps = &material[..32];
                (
                    *key_gen,
                    Generation {
                        content: blake3::derive_key("skillpouch/v1/content", ps),
                        meta: blake3::derive_key("skillpouch/v1/meta", ps),
                    },
                )
            })
            .collect();
        let current = generations.keys().max().copied().unwrap_or(1);
        let pis = &first[32..];
        Ok(Self {
            pouch_id: pouch_id.to_string(),
            current,
            generations,
            blob_id_key: blake3::derive_key("skillpouch/v1/blob-id", pis),
            name_key: blake3::derive_key("skillpouch/v1/name-index", pis),
        })
    }

    /// The generation new writes use.
    pub fn key_gen(&self) -> u32 {
        self.current
    }

    fn generation(&self, key_gen: u32) -> Result<&Generation> {
        self.generations
            .get(&key_gen)
            .ok_or(CryptoError::Format("key generation not available"))
    }

    fn current(&self) -> Result<&Generation> {
        self.generation(self.current)
    }

    /// Same file, same pouch → same id.
    pub fn blob_id(&self, plaintext: &[u8]) -> String {
        b64::encode(blake3::keyed_hash(&self.blob_id_key, plaintext).as_bytes())
    }

    pub fn name_index(&self, kind: &str, scope: &str, name: &str) -> String {
        let message = [
            b"v1".as_slice(),
            kind.as_bytes(),
            &[0],
            scope.as_bytes(),
            &[0],
            name.to_lowercase().as_bytes(),
        ]
        .concat();
        b64::encode(blake3::keyed_hash(&self.name_key, &message).as_bytes())
    }

    fn blob_aad(&self, id: &str) -> Result<Vec<u8>> {
        let id = b64::decode(id).ok_or(CryptoError::Format("bad blob id"))?;
        Ok([
            b"skillpouch/v1/blob".as_slice(),
            self.pouch_id.as_bytes(),
            &id,
        ]
        .concat())
    }

    pub fn seal_blob(&self, plaintext: &[u8]) -> Result<SealedBlob> {
        let id = self.blob_id(plaintext);
        let bytes = sealed::seal(
            &self.current()?.content,
            self.current,
            plaintext,
            &self.blob_aad(&id)?,
        )?;
        Ok(SealedBlob {
            hash: b64::encode(blake3::hash(&bytes).as_bytes()),
            size: plaintext.len() as u64,
            id,
            bytes,
        })
    }

    /// Decrypts a downloaded blob and checks it is the content its id names.
    pub fn open_blob(&self, id: &str, stored: &[u8]) -> Result<Vec<u8>> {
        let key = &self.generation(sealed::key_gen(stored)?)?.content;
        let plaintext = sealed::open(key, stored, &self.blob_aad(id)?)?;
        if self.blob_id(&plaintext) != id {
            return Err(CryptoError::Format("blob content does not match its id"));
        }
        Ok(plaintext)
    }

    fn revision_aad(&self, artifact_id: &str, revision_id: &str) -> Vec<u8> {
        [
            b"skillpouch/v1/revision".as_slice(),
            self.pouch_id.as_bytes(),
            artifact_id.as_bytes(),
            revision_id.as_bytes(),
        ]
        .concat()
    }

    fn signed_message(&self, revision_id: &str, envelope: &[u8]) -> Vec<u8> {
        [
            self.pouch_id.as_bytes(),
            revision_id.as_bytes(),
            blake3::hash(envelope).as_bytes(),
        ]
        .concat()
    }

    pub fn seal_revision(&self, author: &SignKey, body: &RevisionBody) -> Result<SealedRevision> {
        let mut body = body.clone();
        body.entries.sort_by(|a, b| a.path.cmp(&b.path));
        let value = serde_json::to_value(&body).map_err(|_| CryptoError::Format("revision"))?;
        let envelope = sealed::seal(
            &self.current()?.meta,
            self.current,
            canonical_json(&value).as_bytes(),
            &self.revision_aad(&body.artifact_id, &body.revision_id),
        )?;
        Ok(SealedRevision {
            signature: author.sign(
                REVISION_SIG,
                &self.signed_message(&body.revision_id, &envelope),
            ),
            envelope: b64::encode(&envelope),
            key_gen: self.current,
        })
    }

    /// Verifies the author's signature, decrypts, and checks the body
    /// matches the server's columns (so envelopes can't be swapped).
    pub fn open_revision(
        &self,
        columns: &RevisionColumns<'_>,
        signer: &PublicJwk,
    ) -> Result<RevisionBody> {
        let envelope =
            b64::decode(columns.envelope).ok_or(CryptoError::Format("bad revision envelope"))?;
        if !signer.verifies(
            columns.signature,
            REVISION_SIG,
            &self.signed_message(columns.revision_id, &envelope),
        ) {
            return Err(CryptoError::Signature("revision".into()));
        }
        let key = &self.generation(sealed::key_gen(&envelope)?)?.meta;
        let plaintext = sealed::open(
            key,
            &envelope,
            &self.revision_aad(columns.artifact_id, columns.revision_id),
        )?;
        let body: RevisionBody = serde_json::from_slice(&plaintext)
            .map_err(|_| CryptoError::Format("revision is not JSON"))?;
        if body.format != 1 {
            return Err(CryptoError::Format("unknown revision format"));
        }
        // An empty parent column is history the server pruned (older servers
        // emptied the link when the parent went); the signed body keeps it.
        if body.artifact_id != columns.artifact_id
            || body.revision_id != columns.revision_id
            || (columns.parent_id.is_some() && body.parent_id.as_deref() != columns.parent_id)
            || body.author_member_id != columns.signer_member_id
        {
            return Err(CryptoError::Format(
                "revision envelope does not match its record",
            ));
        }
        Ok(body)
    }
}

/// Frames for `blobs/batch`: `id(32) ‖ hash(32) ‖ u32_be(len) ‖ bytes`, repeated.
pub fn frame_batch(blobs: &[&SealedBlob]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for blob in blobs {
        let id = b64::decode(&blob.id).ok_or(CryptoError::Format("bad blob id"))?;
        let hash = b64::decode(&blob.hash).ok_or(CryptoError::Format("bad blob hash"))?;
        let length =
            u32::try_from(blob.bytes.len()).map_err(|_| CryptoError::Format("blob too large"))?;
        out.extend_from_slice(&id);
        out.extend_from_slice(&hash);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&blob.bytes);
    }
    Ok(out)
}

/// Parses a `blobs/fetch` response: `id(32) ‖ u32_be(len) ‖ bytes`, repeated.
pub fn parse_fetched(body: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        if rest.len() < 36 {
            return Err(CryptoError::Format("truncated blob frame"));
        }
        let (id, after) = rest.split_at(32);
        let (length, after) = after.split_at(4);
        let length = u32::from_be_bytes([length[0], length[1], length[2], length[3]]) as usize;
        if after.len() < length {
            return Err(CryptoError::Format("truncated blob"));
        }
        let (bytes, after) = after.split_at(length);
        out.push((b64::encode(id), bytes.to_vec()));
        rest = after;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Result<PouchKeys> {
        let mut gen1 = [1u8; 64];
        gen1[32..].fill(9);
        let mut gen2 = [2u8; 64];
        gen2[32..].fill(9);
        PouchKeys::new(
            "0199aaaa-0000-7000-8000-000000000001",
            &[(1, gen1), (2, gen2)],
        )
    }

    #[test]
    fn blobs_round_trip_and_are_content_addressed() -> Result<()> {
        let keys = keys()?;
        let blob = keys.seal_blob(b"# Skill\n")?;
        assert_eq!(blob.id, keys.blob_id(b"# Skill\n"));
        assert_eq!(sealed::key_gen(&blob.bytes)?, 2);
        assert_eq!(keys.open_blob(&blob.id, &blob.bytes)?, b"# Skill\n");
        let other = keys.seal_blob(b"other")?;
        assert!(keys.open_blob(&other.id, &blob.bytes).is_err());

        let framed = frame_batch(&[&blob])?;
        assert_eq!(framed.len(), 68 + blob.bytes.len());
        let mut fetched = b64::decode(&blob.id).unwrap_or_default();
        fetched.extend_from_slice(&(blob.bytes.len() as u32).to_be_bytes());
        fetched.extend_from_slice(&blob.bytes);
        assert_eq!(parse_fetched(&fetched)?, vec![(blob.id, blob.bytes)]);
        assert!(parse_fetched(&fetched[..40]).is_err());
        Ok(())
    }

    #[test]
    fn names_index_case_insensitively() -> Result<()> {
        let keys = keys()?;
        assert_eq!(
            keys.name_index("skill", "global", "PDF"),
            keys.name_index("skill", "global", "pdf")
        );
        assert_ne!(
            keys.name_index("skill", "global", "pdf"),
            keys.name_index("agent", "global", "pdf")
        );
        Ok(())
    }

    #[test]
    fn revisions_are_signed_and_bound_to_their_record() -> Result<()> {
        let keys = keys()?;
        let author = SignKey::generate()?;
        let body = RevisionBody {
            format: 1,
            artifact_id: "a1".into(),
            revision_id: "r1".into(),
            parent_id: None,
            merge_parent_id: None,
            kind: "skill".into(),
            scope: "global".into(),
            logical_key: "pdf".into(),
            deleted: false,
            entries: vec![ManifestEntry {
                path: "SKILL.md".into(),
                blob: keys.blob_id(b"x"),
                size: 1,
                mode: "file".into(),
            }],
            roster_head: "h".into(),
            author_member_id: "m1".into(),
            created_at: "2026-10-09T00:00:00.000Z".into(),
        };
        let sealed = keys.seal_revision(&author, &body)?;
        let columns = RevisionColumns {
            artifact_id: "a1",
            revision_id: "r1",
            parent_id: None,
            envelope: &sealed.envelope,
            signature: &sealed.signature,
            signer_member_id: "m1",
        };
        let jwk = author.public_jwk()?;
        assert_eq!(keys.open_revision(&columns, &jwk)?, body);
        let moved = RevisionColumns {
            artifact_id: "a2",
            ..columns
        };
        assert!(keys.open_revision(&moved, &jwk).is_err());
        let stranger = SignKey::generate()?.public_jwk()?;
        let columns = RevisionColumns {
            artifact_id: "a1",
            ..moved
        };
        assert!(keys.open_revision(&columns, &stranger).is_err());
        Ok(())
    }
}
