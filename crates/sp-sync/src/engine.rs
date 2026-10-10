//! One sync of a pouch: compare the pouch folder, the server and what both
//! agreed on last time, then send local changes and write remote ones.
//!
//! An item changed on one side only goes the other way. Changed on both
//! sides, the server's version is written to the folder and this computer's
//! is sent on top of the version both last agreed on: the server keeps it as
//! an open conflict that someone resolves in the web app, picking a version.
//! Items two sides bring the first time they meet have no common version;
//! this computer's is kept as `<name>-conflict` then. Nothing is ever lost.

use crate::item::{self, Files, LocalItem, PouchDir, Skipped};
use crate::remote::{Change, Committed, Listing, Remote, RemoteItem};
use crate::state::{Agreed, PouchState};
use crate::{Result, SyncError};
use std::collections::{BTreeMap, BTreeSet};

/// How the first sync of a computer settles items both sides have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Prefer {
    /// The pouch's version stays; this computer's becomes `<name>-conflict`.
    #[default]
    Pouch,
    /// This computer's version replaces the pouch's.
    Computer,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub prefer: Prefer,
    /// Items the pouch has and this computer doesn't are deleted from the
    /// pouch (replacing the pouch with this computer).
    pub delete_unmatched: bool,
}

/// What a sync did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub sent: usize,
    pub received: usize,
    pub conflicts: usize,
    /// Changes the server refused, with its reason.
    pub rejected: Vec<String>,
    /// Items on the server this computer can't verify or decrypt.
    pub unreadable: usize,
    /// Items synced before that the server no longer had (and nobody
    /// deleted): sent again from this computer, by key.
    pub reuploaded: Vec<String>,
    pub skipped: Skipped,
    /// Something changed meanwhile: run again.
    pub again: bool,
}

/// What to do, decided from the three sides without touching anything.
#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub send: Vec<Change>,
    /// Remote items to write into the folder, by key.
    pub receive: Vec<String>,
    /// Folder items the server deleted.
    pub remove: Vec<String>,
    /// Items synced before that the server no longer has, without a signed
    /// delete: sent again (their `Create` is in `send`).
    pub reupload: Vec<String>,
    /// Both sides changed since they agreed: the folder gets the server's,
    /// and the local one goes to the server as a conflict (in `send`).
    pub conflicts: Vec<String>,
    /// Both sides have it, first time they meet, different: the folder gets
    /// the server's, the local one is copied as `<name>-conflict`.
    pub copies: Vec<String>,
    /// Already equal: just remember it.
    pub agree: Vec<String>,
    /// Gone on both sides.
    pub forget: Vec<String>,
}

/// A folder item with its fingerprint as the server would compute it.
pub struct Local {
    pub item: LocalItem,
    pub fingerprint: String,
}

pub fn plan(
    local: &BTreeMap<String, Local>,
    remote: &BTreeMap<String, RemoteItem>,
    state: &PouchState,
    options: Options,
) -> Plan {
    let mut plan = Plan::default();
    let keys: BTreeSet<&String> = local
        .keys()
        .chain(remote.keys())
        .chain(state.items.keys())
        .collect();
    for key in keys {
        let mine = local.get(key);
        let theirs = remote.get(key);
        let live = theirs.filter(|item| !item.deleted);
        let agreed = state.items.get(key);
        match (mine, live) {
            (None, None) => plan.forget.push(key.clone()),
            (Some(mine), Some(theirs)) => {
                if mine.fingerprint == theirs.fingerprint() {
                    plan.agree.push(key.clone());
                    continue;
                }
                let update = || Change::Update {
                    base: theirs.clone(),
                    files: mine.item.files.clone(),
                };
                match agreed {
                    Some(agreed) => {
                        let mine_changed = mine.fingerprint != agreed.fingerprint;
                        let theirs_changed = theirs.revision_id != agreed.revision_id;
                        match (mine_changed, theirs_changed) {
                            (true, false) => plan.send.push(update()),
                            (true, true) => {
                                // On top of the agreed version: a conflict on the server.
                                let mut base = theirs.clone();
                                base.revision_id.clone_from(&agreed.revision_id);
                                plan.send.push(Change::Update {
                                    base,
                                    files: mine.item.files.clone(),
                                });
                                plan.conflicts.push(key.clone());
                            }
                            (false, _) => plan.receive.push(key.clone()),
                        }
                    }
                    None if options.prefer == Prefer::Computer => plan.send.push(update()),
                    None => plan.copies.push(key.clone()),
                }
            }
            (Some(mine), None) => match theirs {
                // Deleted on the server (a signed delete), untouched here.
                Some(_) if agreed.is_some_and(|a| a.fingerprint == mine.fingerprint) => {
                    plan.remove.push(key.clone());
                }
                Some(base) => plan.send.push(Change::Restore {
                    base: base.clone(),
                    files: mine.item.files.clone(),
                }),
                // Not on the server at all, and nobody signed its deletion:
                // a server restored from a backup, a replica behind, or one
                // hiding it. Only a signed delete removes an item here, so
                // this computer sends its copy back.
                None => {
                    if agreed.is_some() {
                        plan.reupload.push(key.clone());
                    }
                    plan.send.push(Change::Create {
                        kind: mine.item.kind.clone(),
                        name: mine.item.name.clone(),
                        files: mine.item.files.clone(),
                    });
                }
            },
            (None, Some(theirs)) => {
                let deleted_here = agreed.is_some_and(|a| a.revision_id == theirs.revision_id);
                let first_meeting = agreed.is_none() && options.delete_unmatched;
                if deleted_here || first_meeting {
                    plan.send.push(Change::Delete {
                        base: theirs.clone(),
                    });
                } else {
                    plan.receive.push(key.clone());
                }
            }
        }
    }
    plan
}

/// `pdf-conflict`, or `pdf-conflict-2` when that's taken too.
pub fn conflict_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let stem: String = name.chars().take(64 - "-conflict-99".len()).collect();
    let mut candidate = format!("{stem}-conflict");
    let mut number = 2;
    while taken(&candidate) && number < 100 {
        candidate = format!("{stem}-conflict-{number}");
        number += 1;
    }
    candidate
}

/// Syncs `dir` with the server once.
pub async fn sync(
    remote: &Remote<'_>,
    dir: &PouchDir,
    state: &mut PouchState,
    options: Options,
) -> Result<Outcome> {
    let high_water = remote.pouch.event_seq;
    let listing = remote.list().await?;
    // An older head than one seen before is a server replaying the past:
    // nothing of this listing is trusted, nothing here changes.
    if let Some(head) = listing
        .heads
        .iter()
        .find(|head| state.is_rollback(&head.artifact_id, &head.revision_id))
    {
        return Err(SyncError::Rollback(head.key.clone()));
    }
    for head in &listing.heads {
        state.saw(
            &head.artifact_id,
            &head.revision_id,
            head.parents.iter().map(String::as_str),
        );
    }
    let Listing {
        items: remote_items,
        unreadable,
        ..
    } = listing;
    let (folder, skipped) = dir.read()?;
    // An item whose server version doesn't verify is left alone on both
    // sides: a tampered head must not delete or replace it here.
    let held: BTreeSet<String> = state
        .items
        .iter()
        .filter(|(_, agreed)| unreadable.contains(&agreed.artifact_id))
        .map(|(key, _)| key.clone())
        .collect();
    let local: BTreeMap<String, Local> = folder
        .into_iter()
        .filter(|(key, _)| !held.contains(key))
        .map(|(key, item)| {
            let fingerprint = remote.local_fingerprint(&item.files);
            (key, Local { item, fingerprint })
        })
        .collect();
    let mut planned = state.clone();
    planned.items.retain(|key, _| !held.contains(key));
    let plan = plan(&local, &remote_items, &planned, options);
    let mut outcome = Outcome {
        unreadable: unreadable.len(),
        skipped,
        ..Outcome::default()
    };

    for key in &plan.forget {
        state.items.remove(key);
    }
    for key in &plan.agree {
        if let Some(theirs) = remote_items.get(key) {
            state
                .items
                .insert(key.clone(), agreed(theirs, theirs.fingerprint()));
        }
    }

    // First meetings: keep this computer's copy under a new name first.
    for key in &plan.copies {
        let Some(mine) = local.get(key) else { continue };
        let taken = |name: &str| {
            local.contains_key(&item::key(&mine.item.kind, name))
                || remote_items.contains_key(&item::key(&mine.item.kind, name))
        };
        let copy = conflict_name(&mine.item.name, &taken);
        dir.write_item(&mine.item.kind, &copy, &mine.item.files)?;
        outcome.conflicts += 1;
        outcome.again = true;
    }

    if !plan.send.is_empty() {
        let results = remote.commit(&plan.send).await?;
        for (change, result) in plan.send.iter().zip(results) {
            let key = change.key();
            match result {
                Committed::Applied {
                    artifact_id,
                    revision_id,
                } => {
                    outcome.sent += 1;
                    if plan.reupload.contains(&key) {
                        outcome.reuploaded.push(key.clone());
                    }
                    let base = match change {
                        Change::Create { .. } => None,
                        Change::Update { base, .. }
                        | Change::Restore { base, .. }
                        | Change::Delete { base } => Some(base.revision_id.as_str()),
                    };
                    state.saw(&artifact_id, &revision_id, base);
                    match change {
                        Change::Delete { .. } => {
                            state.items.remove(&key);
                        }
                        Change::Create { kind, name, files } => {
                            let fingerprint = remote.local_fingerprint(files);
                            state.items.insert(
                                key,
                                Agreed {
                                    artifact_id,
                                    revision_id,
                                    kind: kind.clone(),
                                    name: name.clone(),
                                    fingerprint,
                                },
                            );
                        }
                        Change::Update { base, files } | Change::Restore { base, files } => {
                            let fingerprint = remote.local_fingerprint(files);
                            state.items.insert(
                                key,
                                Agreed {
                                    artifact_id,
                                    revision_id,
                                    kind: base.kind.clone(),
                                    name: base.name.clone(),
                                    fingerprint,
                                },
                            );
                        }
                    }
                }
                // Ours was stale: the server keeps it as an open conflict.
                Committed::Conflict if plan.conflicts.contains(&key) => outcome.conflicts += 1,
                Committed::Conflict => outcome.again = true,
                Committed::Rejected(reason) => outcome.rejected.push(format!("{key}: {reason}")),
            }
        }
    }

    let incoming: Vec<&RemoteItem> = plan
        .receive
        .iter()
        .chain(&plan.conflicts)
        .chain(&plan.copies)
        .filter_map(|key| remote_items.get(key))
        .collect();
    if !incoming.is_empty() {
        let files = remote.download(&incoming).await?;
        for theirs in incoming {
            let Some(files) = files.get(&theirs.artifact_id) else {
                continue;
            };
            let key = theirs.key();
            // Edited while we downloaded: leave it, the next sync sends it.
            if changed_since(dir, local.get(&key), &theirs.kind, &theirs.name, remote)? {
                outcome.again = true;
                continue;
            }
            if let Some(mine) = local.get(&key)
                && mine.item.name != theirs.name
            {
                dir.remove_item(&mine.item.kind, &mine.item.name)?;
            }
            dir.write_item(&theirs.kind, &theirs.name, files)?;
            state
                .items
                .insert(key, agreed(theirs, theirs.fingerprint()));
            outcome.received += 1;
        }
    }

    for key in &plan.remove {
        let Some(mine) = local.get(key) else { continue };
        if changed_since(dir, Some(mine), &mine.item.kind, &mine.item.name, remote)? {
            outcome.again = true;
            continue;
        }
        dir.remove_item(&mine.item.kind, &mine.item.name)?;
        state.items.remove(key);
        outcome.received += 1;
    }

    state.epoch.clone_from(&remote.pouch.epoch);
    if !outcome.again {
        state.cursor = high_water;
    }
    Ok(outcome)
}

fn agreed(theirs: &RemoteItem, fingerprint: String) -> Agreed {
    Agreed {
        artifact_id: theirs.artifact_id.clone(),
        revision_id: theirs.revision_id.clone(),
        kind: theirs.kind.clone(),
        name: theirs.name.clone(),
        fingerprint,
    }
}

/// The folder item no longer matches what was read at the start.
fn changed_since(
    dir: &PouchDir,
    read: Option<&Local>,
    kind: &str,
    name: &str,
    remote: &Remote<'_>,
) -> Result<bool> {
    let Some(path) = dir.item_dir(kind, name) else {
        return Ok(false);
    };
    let now: Files = item::read_files(&path, "", &mut Skipped::default())?;
    Ok(match read {
        Some(read) => remote.local_fingerprint(&now) != read.fingerprint,
        None => !now.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_crypto::pouch::{ManifestEntry, RevisionBody};

    fn remote_item(name: &str, revision: &str, blob: &str, deleted: bool) -> RemoteItem {
        let entries = vec![ManifestEntry {
            path: "SKILL.md".into(),
            blob: blob.into(),
            size: 1,
            mode: "file".into(),
        }];
        RemoteItem {
            artifact_id: format!("a-{name}"),
            revision_id: revision.into(),
            kind: "skill".into(),
            name: name.into(),
            entries: entries.clone(),
            deleted,
            body: RevisionBody {
                format: 1,
                artifact_id: format!("a-{name}"),
                revision_id: revision.into(),
                parent_id: None,
                merge_parent_id: None,
                kind: "skill".into(),
                scope: "global".into(),
                logical_key: name.into(),
                deleted,
                entries,
                roster_head: String::new(),
                author_member_id: String::new(),
                created_at: String::new(),
            },
        }
    }

    /// A local item whose single file has content id `blob`.
    fn local_item(name: &str, blob: &str) -> Local {
        let mut files = Files::new();
        files.insert("SKILL.md".into(), item::File::new(blob));
        Local {
            item: LocalItem {
                kind: "skill".into(),
                name: name.into(),
                files,
            },
            fingerprint: crate::remote::fingerprint([("SKILL.md", blob, false)].into_iter()),
        }
    }

    fn agreed_on(name: &str, revision: &str, blob: &str) -> (String, Agreed) {
        (
            item::key("skill", name),
            Agreed {
                artifact_id: format!("a-{name}"),
                revision_id: revision.into(),
                kind: "skill".into(),
                name: name.into(),
                fingerprint: crate::remote::fingerprint([("SKILL.md", blob, false)].into_iter()),
            },
        )
    }

    fn sides(
        local: &[Local],
        remote: &[RemoteItem],
        agreed: &[(String, Agreed)],
    ) -> (
        BTreeMap<String, Local>,
        BTreeMap<String, RemoteItem>,
        PouchState,
    ) {
        let local = local
            .iter()
            .map(|l| {
                (
                    item::key(&l.item.kind, &l.item.name),
                    Local {
                        item: l.item.clone(),
                        fingerprint: l.fingerprint.clone(),
                    },
                )
            })
            .collect();
        let remote = remote.iter().map(|r| (r.key(), r.clone())).collect();
        let state = PouchState {
            items: agreed.iter().cloned().collect(),
            ..PouchState::default()
        };
        (local, remote, state)
    }

    fn kinds(plan: &Plan) -> Vec<String> {
        plan.send
            .iter()
            .map(|change| match change {
                Change::Create { name, .. } => format!("create {name}"),
                Change::Update { base, .. } => format!("update {}", base.name),
                Change::Restore { base, .. } => format!("restore {}", base.name),
                Change::Delete { base } => format!("delete {}", base.name),
            })
            .collect()
    }

    #[test]
    fn one_sided_changes_go_the_other_way() {
        let (local, remote, state) = sides(
            &[
                local_item("edited", "v2"),
                local_item("new", "n"),
                local_item("same", "s"),
                local_item("stale", "v1"),
            ],
            &[
                remote_item("edited", "r1", "v1", false),
                remote_item("same", "r1", "s", false),
                remote_item("stale", "r2", "v2", false),
                remote_item("fresh", "r1", "f", false),
                remote_item("gone-here", "r1", "g", false),
            ],
            &[
                agreed_on("edited", "r1", "v1"),
                agreed_on("stale", "r1", "v1"),
                agreed_on("gone-here", "r1", "g"),
            ],
        );
        let plan = plan(&local, &remote, &state, Options::default());
        assert_eq!(
            kinds(&plan),
            ["update edited", "delete gone-here", "create new"]
        );
        assert_eq!(plan.receive, ["skill/fresh", "skill/stale"]);
        assert_eq!(plan.agree, ["skill/same"]);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn both_sides_changed_is_a_conflict_unless_the_computer_wins() {
        let (local, remote, state) = sides(
            &[local_item("pdf", "mine")],
            &[remote_item("pdf", "r2", "theirs", false)],
            &[agreed_on("pdf", "r1", "old")],
        );
        let plan_ = plan(&local, &remote, &state, Options::default());
        assert_eq!(plan_.conflicts, ["skill/pdf"]);
        // Sent on top of the agreed version, so the server opens a conflict.
        let [Change::Update { base, .. }] = plan_.send.as_slice() else {
            panic!("expected one update, got {}", plan_.send.len());
        };
        assert_eq!(base.revision_id, "r1");

        // First meeting (no agreement yet): a `-conflict` copy by default...
        let (local, remote, state) = sides(
            &[local_item("pdf", "mine")],
            &[remote_item("pdf", "r2", "theirs", false)],
            &[],
        );
        assert_eq!(
            plan(&local, &remote, &state, Options::default()).copies,
            ["skill/pdf"]
        );
        // ...or this computer's version when it replaces the pouch.
        let replace = Options {
            prefer: Prefer::Computer,
            delete_unmatched: true,
        };
        let (local, remote, state) = sides(
            &[local_item("pdf", "mine")],
            &[
                remote_item("pdf", "r2", "theirs", false),
                remote_item("extra", "r1", "x", false),
            ],
            &[],
        );
        assert_eq!(
            kinds(&plan(&local, &remote, &state, replace)),
            ["delete extra", "update pdf"]
        );
    }

    #[test]
    fn deletions_follow_the_side_that_deleted() {
        let (local, remote, state) = sides(
            &[
                local_item("removed-there", "x"),
                local_item("edited-here", "new"),
            ],
            &[
                remote_item("removed-there", "r2", "x", true),
                remote_item("edited-here", "r2", "x", true),
            ],
            &[
                agreed_on("removed-there", "r1", "x"),
                agreed_on("edited-here", "r1", "x"),
            ],
        );
        let plan = plan(&local, &remote, &state, Options::default());
        assert_eq!(plan.remove, ["skill/removed-there"]);
        assert_eq!(kinds(&plan), ["restore edited-here"]);
    }

    #[test]
    fn only_a_signed_delete_removes_an_item_here() {
        // Both synced before; the server lists one as deleted and the other
        // not at all.
        let (local, remote, state) = sides(
            &[local_item("deleted", "x"), local_item("missing", "y")],
            &[remote_item("deleted", "r2", "x", true)],
            &[
                agreed_on("deleted", "r1", "x"),
                agreed_on("missing", "r1", "y"),
            ],
        );
        let plan = plan(&local, &remote, &state, Options::default());
        assert_eq!(plan.remove, ["skill/deleted"]);
        assert_eq!(kinds(&plan), ["create missing"]);
        assert_eq!(plan.reupload, ["skill/missing"]);
    }

    #[test]
    fn conflict_copies_get_a_free_name() {
        assert_eq!(conflict_name("pdf", &|_| false), "pdf-conflict");
        assert_eq!(
            conflict_name("pdf", &|n| n == "pdf-conflict"),
            "pdf-conflict-2"
        );
        assert!(conflict_name(&"a".repeat(64), &|_| false).len() <= 64);
    }
}
