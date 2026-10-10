//! The pouch on the server: its items decrypted and verified, and commits of
//! new revisions sealed and signed by this computer.

use crate::item::{self, File, Files};
use crate::state::RosterPin;
use crate::{Result, SyncError};
use sp_api::Client;
use sp_api::types::{
    BlobCheck, CommitRequest, Operation, OperationResult, PouchSummary, RevisionRecord,
};
use sp_core::{Credentials, Home};
use sp_crypto::pouch::{
    ManifestEntry, PouchKeys, RevisionBody, RevisionColumns, SealedBlob, frame_batch, parse_fetched,
};
use sp_crypto::vault::{self, Welcome};
use sp_crypto::{CryptoError, EncKey, SignKey};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The only scope synced into an assistant's home folders.
pub const GLOBAL: &str = "global";

/// Blobs per check / fetch / upload request and upload body size (API limits).
const BLOB_CHECK_MAX: usize = 1000;
const BLOB_BATCH_PARTS: usize = 256;
const BLOB_BATCH_BYTES: usize = 8 * 1024 * 1024;
/// Operations per commit: small enough that a commit body stays well under
/// the server's limit even with large manifests.
const OPS_PER_COMMIT: usize = 20;

/// An item's head revision on the server, opened.
#[derive(Debug, Clone)]
pub struct RemoteItem {
    pub artifact_id: String,
    pub revision_id: String,
    pub kind: String,
    pub name: String,
    pub entries: Vec<ManifestEntry>,
    pub deleted: bool,
    pub body: RevisionBody,
}

impl RemoteItem {
    pub fn key(&self) -> String {
        item::key(&self.kind, &self.name)
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(self.entries.iter().map(|entry| {
            (
                entry.path.as_str(),
                entry.blob.as_str(),
                entry.mode == "exec",
            )
        }))
    }
}

/// A verified head revision of any item, synced here or not.
#[derive(Debug, Clone)]
pub struct Head {
    pub artifact_id: String,
    pub revision_id: String,
    pub parent_id: Option<String>,
    /// Parent and merge parent, as the signed body names them.
    pub parents: Vec<String>,
    /// `kind/name`, for messages.
    pub key: String,
}

/// The pouch's items as the server lists them.
#[derive(Debug, Default)]
pub struct Listing {
    /// Global items, deleted ones included, by key.
    pub items: BTreeMap<String, RemoteItem>,
    /// Every head that verified.
    pub heads: Vec<Head>,
    /// Artifacts whose head doesn't verify: listed by id, never trusted.
    pub unreadable: BTreeSet<String>,
}

/// Identifies a set of files by path, content id and exec bit: equal on
/// both sides means nothing to sync.
pub fn fingerprint<'a>(files: impl Iterator<Item = (&'a str, &'a str, bool)>) -> String {
    let mut lines: Vec<String> = files
        .map(|(path, blob, exec)| format!("{path}\0{blob}\0{}\n", u8::from(exec)))
        .collect();
    lines.sort();
    blake3::hash(lines.concat().as_bytes()).to_hex().to_string()
}

/// One change to send.
#[derive(Debug, Clone)]
pub enum Change {
    Create {
        /// Reuse a previously agreed artifact id when restoring a head that
        /// disappeared without a signed tombstone.
        artifact_id: Option<String>,
        kind: String,
        name: String,
        files: Files,
    },
    Update {
        base: RemoteItem,
        files: Files,
    },
    /// Brings back a deleted item with new files.
    Restore {
        base: RemoteItem,
        files: Files,
    },
    Delete {
        base: RemoteItem,
    },
}

impl Change {
    pub fn key(&self) -> String {
        match self {
            Self::Create { kind, name, .. } => item::key(kind, name),
            Self::Update { base, .. } | Self::Restore { base, .. } | Self::Delete { base } => {
                base.key()
            }
        }
    }
}

/// What the server did with one change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Committed {
    Applied {
        artifact_id: String,
        revision_id: String,
    },
    /// Someone changed it first: sync again.
    Conflict,
    Rejected(String),
}

pub struct Remote<'a> {
    client: &'a Client,
    key: &'a SignKey,
    token: String,
    pub pouch: PouchSummary,
    keys: PouchKeys,
    roster: vault::Roster,
    member_id: String,
}

impl<'a> Remote<'a> {
    /// Opens `pouch_id` for this computer: verifies the roster against the
    /// welcome and the newest roster it verified before (`state/roster.json`
    /// in `home`), then opens the pouch's key envelopes.
    pub async fn open(
        client: &'a Client,
        key: &'a SignKey,
        token: &str,
        credentials: &Credentials,
        pouch_id: &str,
        home: &Home,
    ) -> Result<Remote<'a>> {
        let member_id = credentials.member_id.clone().ok_or_else(|| {
            SyncError::Invalid(
                "this computer logged in before syncing existed: log in again to sync".into(),
            )
        })?;
        let enc = EncKey::from_b64(&credentials.enc_key)?;
        let welcome = vault::open_welcome(&enc, &member_id, &credentials.welcome)?;
        let pinned = RosterPin::load(home)?.filter(|pin| {
            pin.genesis_hash == welcome.genesis_hash
                && pin.member_id == member_id
                && pin.head_seq > welcome.roster_head_seq
        });
        let trusted = match &pinned {
            Some(pin) => Welcome {
                roster_head_seq: pin.head_seq,
                roster_head_hash: pin.head_hash.clone(),
                ..welcome.clone()
            },
            None => welcome.clone(),
        };
        let pouches = client.pouches(key, token).await;
        let roster = client.roster(key, token).await;
        let envelopes = client.key_envelopes(key, token).await;
        // A deleted pouch is neither written into nor taken off this
        // computer: its items there stay as they are.
        let pouch = pouches?
            .pouches
            .into_iter()
            .find(|pouch| pouch.id == pouch_id && pouch.state != "deleted")
            .ok_or(SyncError::PouchDeleted)?;
        let roster = vault::verify_roster(&roster?.statements, &trusted)?;
        if pinned
            .as_ref()
            .is_none_or(|pin| roster.head_seq > pin.head_seq)
        {
            RosterPin {
                genesis_hash: welcome.genesis_hash.clone(),
                member_id: member_id.clone(),
                head_seq: roster.head_seq,
                head_hash: roster.head_hash.clone(),
            }
            .save(home)?;
        }
        let announced = roster
            .pouches
            .get(pouch_id)
            .map(|pouch| pouch.key_gen)
            .ok_or_else(|| CryptoError::Roster("the pouch is not in the account roster".into()))?;
        let opened: Vec<(u32, [u8; 64])> =
            vault::open_envelopes(&roster, &envelopes?.key_envelopes, &member_id, &enc)?
                .into_iter()
                .filter(|((id, _), _)| id == pouch_id)
                .map(|((_, key_gen), material)| (key_gen, material))
                .collect();
        let keys = PouchKeys::new(pouch_id, &opened)?;
        if keys.key_gen() < announced {
            // Writing with an older key would let a removed member read it.
            return Err(CryptoError::Roster(format!(
                "the server withheld key generation {announced} of this pouch"
            ))
            .into());
        }
        if keys.key_gen() != pouch.key_gen && pouch.key_gen != 0 {
            return Err(SyncError::Invalid(
                "this computer doesn't have the pouch's newest key yet; open the web app once to share it".into(),
            ));
        }
        Ok(Remote {
            client,
            key,
            token: token.to_string(),
            pouch,
            keys,
            roster,
            member_id,
        })
    }

    pub fn keys(&self) -> &PouchKeys {
        &self.keys
    }

    /// Every item's head, deleted ones included.
    pub async fn list(&self) -> Result<Listing> {
        let mut listing = Listing::default();
        let mut cursor: Option<String> = None;
        loop {
            let (page, next) = self
                .client
                .artifact_page(self.key, &self.token, &self.pouch.id, cursor.as_deref())
                .await?;
            for artifact in page {
                let Some(head) = artifact.head else { continue };
                let Ok(body) = self.open_head(&head) else {
                    listing.unreadable.insert(artifact.id);
                    continue;
                };
                listing.heads.push(Head {
                    artifact_id: body.artifact_id.clone(),
                    revision_id: body.revision_id.clone(),
                    parent_id: body.parent_id.clone(),
                    parents: body
                        .parent_id
                        .iter()
                        .chain(&body.merge_parent_id)
                        .cloned()
                        .collect(),
                    key: item::key(&body.kind, &body.logical_key),
                });
                if body.scope != GLOBAL
                    || item::folder_of(&body.kind).is_none()
                    || !item::valid_name(&body.logical_key)
                {
                    continue;
                }
                let item = RemoteItem {
                    artifact_id: body.artifact_id.clone(),
                    revision_id: body.revision_id.clone(),
                    kind: body.kind.clone(),
                    name: body.logical_key.clone(),
                    entries: body.entries.clone(),
                    // Server-side visibility metadata is unsigned. Only the
                    // verified revision body can authorize deleting local data.
                    deleted: body.deleted,
                    body,
                };
                let key = item.key();
                // A live item beats a deleted one with the same name.
                if listing
                    .items
                    .get(&key)
                    .is_none_or(|existing| existing.deleted)
                {
                    listing.items.insert(key, item);
                }
            }
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(listing)
    }

    /// Verifies a head revision and opens its body.
    fn open_head(&self, head: &RevisionRecord) -> Result<RevisionBody> {
        let signer = self
            .roster
            .members
            .get(&head.signer_member_id)
            .ok_or_else(|| CryptoError::Signature("revision by an unknown member".into()))?;
        let body = self.keys.open_revision(
            &RevisionColumns {
                artifact_id: &head.artifact_id,
                revision_id: &head.id,
                parent_id: head.parent_id.as_deref(),
                envelope: &head.envelope,
                signature: &head.signature,
                signer_member_id: &head.signer_member_id,
            },
            &signer.sign_pub,
        )?;
        if !self
            .roster
            .may_sign_revision(&head.signer_member_id, &body.roster_head)
        {
            return Err(CryptoError::Signature(
                "revision by a member removed before it was written".into(),
            )
            .into());
        }
        Ok(body)
    }

    /// The files of these items, downloaded and decrypted, by artifact id.
    pub async fn download(&self, items: &[&RemoteItem]) -> Result<HashMap<String, Files>> {
        let mut ids: Vec<String> = items
            .iter()
            .flat_map(|item| item.entries.iter().map(|entry| entry.blob.clone()))
            .collect();
        ids.sort();
        ids.dedup();
        let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
        for batch in ids.chunks(BLOB_BATCH_PARTS) {
            let framed = self
                .client
                .fetch_blobs(self.key, &self.token, &self.pouch.id, batch)
                .await?;
            for (id, stored) in parse_fetched(&framed)? {
                let plaintext = self.keys.open_blob(&id, &stored)?;
                blobs.insert(id, plaintext);
            }
        }
        let mut out = HashMap::new();
        for item in items {
            let mut files = Files::new();
            for entry in &item.entries {
                if !item::safe_path(&entry.path) {
                    return Err(SyncError::Invalid(format!(
                        "{} has an unsafe file path: {}",
                        item.name, entry.path
                    )));
                }
                let bytes = blobs.get(&entry.blob).cloned().ok_or_else(|| {
                    SyncError::Invalid(format!("{} is missing {}", item.name, entry.path))
                })?;
                files.insert(
                    entry.path.clone(),
                    File {
                        bytes,
                        exec: entry.mode == "exec",
                    },
                );
            }
            out.insert(item.artifact_id.clone(), files);
        }
        Ok(out)
    }

    /// What `files` will be on the server (no network).
    pub fn local_fingerprint(&self, files: &Files) -> String {
        let ids: Vec<(String, String, bool)> = files
            .iter()
            .map(|(path, file)| (path.clone(), self.keys.blob_id(&file.bytes), file.exec))
            .collect();
        fingerprint(ids.iter().map(|(p, b, e)| (p.as_str(), b.as_str(), *e)))
    }

    /// Sends `changes`; one result per change, in order.
    pub async fn commit(&self, changes: &[Change]) -> Result<Vec<Committed>> {
        let mut results = Vec::with_capacity(changes.len());
        for chunk in changes.chunks(OPS_PER_COMMIT) {
            let mut blobs: Vec<SealedBlob> = Vec::new();
            let mut ops = Vec::with_capacity(chunk.len());
            for change in chunk {
                let (op, sealed) = self.operation(change)?;
                blobs.extend(sealed);
                ops.push(op);
            }
            self.upload(blobs).await?;
            let request = CommitRequest {
                operation_id: uuid::Uuid::new_v4().to_string(),
                epoch: self.pouch.epoch.clone(),
                ops,
            };
            let response = self
                .client
                .commit(self.key, &self.token, &self.pouch.id, &request)
                .await?;
            for (op, result) in request.ops.iter().zip(response.results) {
                results.push(match result {
                    OperationResult::Applied { .. } => Committed::Applied {
                        artifact_id: op.artifact_id.clone(),
                        revision_id: op.revision_id.clone(),
                    },
                    OperationResult::Conflict { .. } => Committed::Conflict,
                    OperationResult::Rejected { message, .. } => Committed::Rejected(message),
                });
            }
        }
        Ok(results)
    }

    fn operation(&self, change: &Change) -> Result<(Operation, Vec<SealedBlob>)> {
        let (kind, artifact_id, base, name, files, deleted) = match change {
            Change::Create {
                artifact_id,
                kind,
                name,
                files,
            } => (
                kind.as_str(),
                artifact_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                None,
                name.as_str(),
                Some(files),
                false,
            ),
            Change::Update { base, files } | Change::Restore { base, files } => (
                base.kind.as_str(),
                base.artifact_id.clone(),
                Some(base),
                base.name.as_str(),
                Some(files),
                false,
            ),
            Change::Delete { base } => (
                base.kind.as_str(),
                base.artifact_id.clone(),
                Some(base),
                base.name.as_str(),
                None,
                true,
            ),
        };
        let mut sealed = Vec::new();
        let entries = match files {
            Some(files) => files
                .iter()
                .map(|(path, file)| {
                    let blob = self.keys.seal_blob(&file.bytes)?;
                    let entry = ManifestEntry {
                        path: path.clone(),
                        blob: blob.id.clone(),
                        size: blob.size,
                        mode: if file.exec { "exec" } else { "file" }.into(),
                    };
                    sealed.push(blob);
                    Ok(entry)
                })
                .collect::<Result<Vec<_>>>()?,
            None => base.map(|base| base.entries.clone()).unwrap_or_default(),
        };
        let revision_id = uuid::Uuid::new_v4().to_string();
        let body = RevisionBody {
            format: 1,
            artifact_id: artifact_id.clone(),
            revision_id: revision_id.clone(),
            parent_id: base.map(|base| base.revision_id.clone()),
            merge_parent_id: None,
            kind: kind.to_string(),
            scope: GLOBAL.into(),
            logical_key: name.to_string(),
            deleted,
            entries,
            roster_head: self.roster.head_hash.clone(),
            author_member_id: self.member_id.clone(),
            created_at: iso_now(),
        };
        let revision = self.keys.seal_revision(self.key, &body)?;
        let op_type = match change {
            Change::Create { .. } => "create",
            Change::Update { .. } => "update",
            Change::Restore { .. } => "restore",
            Change::Delete { .. } => "delete",
        };
        let names_itself = matches!(change, Change::Create { .. } | Change::Restore { .. });
        let mut blob_ids: Vec<String> = body.entries.iter().map(|e| e.blob.clone()).collect();
        blob_ids.sort();
        blob_ids.dedup();
        Ok((
            Operation {
                kind: op_type,
                artifact_id,
                revision_id,
                base_revision_id: base.map(|base| base.revision_id.clone()),
                merge_parent_id: None,
                name_index: names_itself.then(|| self.keys.name_index(kind, GLOBAL, name)),
                conflict_id: None,
                key_gen: revision.key_gen,
                envelope: revision.envelope,
                signature: revision.signature,
                signer_member_id: self.member_id.clone(),
                blob_ids,
            },
            sealed,
        ))
    }

    async fn upload(&self, blobs: Vec<SealedBlob>) -> Result<()> {
        let mut unique: BTreeMap<String, SealedBlob> = BTreeMap::new();
        for blob in blobs {
            unique.entry(blob.id.clone()).or_insert(blob);
        }
        let unique: Vec<SealedBlob> = unique.into_values().collect();
        let mut missing = std::collections::HashSet::new();
        for chunk in unique.chunks(BLOB_CHECK_MAX) {
            let checks: Vec<BlobCheck> = chunk
                .iter()
                .map(|blob| BlobCheck {
                    id: blob.id.clone(),
                    stored_size: blob.bytes.len() as u64,
                })
                .collect();
            let answer = self
                .client
                .check_blobs(self.key, &self.token, &self.pouch.id, &checks)
                .await?;
            missing.extend(answer.missing);
        }
        let mut batch: Vec<&SealedBlob> = Vec::new();
        let mut batch_bytes = 0;
        for blob in unique.iter().filter(|blob| missing.contains(&blob.id)) {
            let size = blob.bytes.len() + 68;
            if !batch.is_empty()
                && (batch.len() == BLOB_BATCH_PARTS || batch_bytes + size > BLOB_BATCH_BYTES)
            {
                self.send_batch(&batch).await?;
                batch.clear();
                batch_bytes = 0;
            }
            batch.push(blob);
            batch_bytes += size;
        }
        if !batch.is_empty() {
            self.send_batch(&batch).await?;
        }
        Ok(())
    }

    async fn send_batch(&self, batch: &[&SealedBlob]) -> Result<()> {
        self.client
            .upload_blobs(self.key, &self.token, &self.pouch.id, frame_batch(batch)?)
            .await?;
        Ok(())
    }

    /// How many conflicts wait to be resolved in the web app.
    pub async fn open_conflicts(&self) -> Result<usize> {
        Ok(self
            .client
            .open_conflicts(self.key, &self.token, &self.pouch.id)
            .await?
            .conflicts
            .len())
    }

    /// Tells the server this computer has everything up to `seq`.
    pub async fn acknowledge(&self, seq: u64) -> Result<()> {
        Ok(self
            .client
            .report_cursor(
                self.key,
                &self.token,
                &self.pouch.id,
                &self.pouch.epoch,
                seq,
            )
            .await?)
    }
}

/// `2026-10-09T12:00:00.000Z`, like JavaScript's `toISOString`.
pub fn iso_now() -> String {
    let now = OffsetDateTime::now_utc();
    let millis = now.millisecond();
    let seconds = now
        .replace_millisecond(0)
        .unwrap_or(now)
        .format(&Rfc3339)
        .unwrap_or_default();
    seconds.replace('Z', &format!(".{millis:03}Z"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_ignore_order_but_not_content_or_mode() {
        let a = fingerprint([("a", "1", false), ("b", "2", false)].into_iter());
        let b = fingerprint([("b", "2", false), ("a", "1", false)].into_iter());
        assert_eq!(a, b);
        assert_ne!(
            a,
            fingerprint([("a", "1", true), ("b", "2", false)].into_iter())
        );
        assert_ne!(
            a,
            fingerprint([("a", "3", false), ("b", "2", false)].into_iter())
        );
    }

    #[test]
    fn timestamps_look_like_javascript() {
        let now = iso_now();
        assert_eq!(now.len(), 24, "{now}");
        assert!(now.ends_with('Z') && now.contains('.'), "{now}");
    }
}
