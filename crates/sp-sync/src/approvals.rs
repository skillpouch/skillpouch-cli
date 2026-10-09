//! Hooks and plugins run code on this computer by themselves, so one that
//! arrives from elsewhere (the web, another computer, someone with access to
//! the account) only reaches the assistants after it's approved here. What
//! this computer brought in itself counts as approved.
//!
//! `state/approved.json` holds, per item, the version approved. A hook
//! changed elsewhere keeps running as approved until the change is.

use crate::Result;
use crate::hooks::{self, Hook};
use crate::item::{self, LocalItem};
use serde::{Deserialize, Serialize};
use sp_core::Home;
use sp_core::home::{read_optional, write_private};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The kinds that need approval.
pub const KINDS: [&str; 2] = ["hook", "plugin"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub fingerprint: String,
    /// The approved hook, to keep running while a newer one waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<Hook>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approvals {
    /// By item key (`hook/format`).
    #[serde(default)]
    pub items: BTreeMap<String, Approval>,
}

/// One item waiting for approval, in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    pub kind: String,
    pub name: String,
    /// What it runs.
    pub detail: String,
    /// An older version runs meanwhile.
    #[serde(default)]
    pub changed: bool,
}

/// Identifies an item's content.
pub fn fingerprint(item: &LocalItem) -> String {
    let mut hasher = blake3::Hasher::new();
    for (path, file) in &item.files {
        hasher.update(path.as_bytes());
        hasher.update(&[0, u8::from(file.exec)]);
        hasher.update(&(file.bytes.len() as u64).to_be_bytes());
        hasher.update(&file.bytes);
    }
    hasher.finalize().to_hex().to_string()
}

fn needs_approval(item: &LocalItem) -> bool {
    KINDS.contains(&item.kind.as_str())
}

impl Approvals {
    fn path(home: &Home) -> PathBuf {
        home.state_dir().join("approved.json")
    }

    pub fn load(home: &Home) -> Self {
        read_optional(&Self::path(home))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        home.ensure_private_dir(&home.state_dir())?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| crate::SyncError::Invalid(error.to_string()))?;
        Ok(write_private(&Self::path(home), &bytes)?)
    }

    pub fn approve(&mut self, item: &LocalItem) {
        if !needs_approval(item) {
            return;
        }
        let hook = (item.kind == "hook")
            .then(|| item.files.get("hook.json"))
            .flatten()
            .and_then(|file| hooks::parse_hook(&file.bytes).ok());
        self.items.insert(
            item::key(&item.kind, &item.name),
            Approval {
                fingerprint: fingerprint(item),
                hook,
            },
        );
    }

    pub fn approved(&self, item: &LocalItem) -> bool {
        !needs_approval(item)
            || self
                .items
                .get(&item::key(&item.kind, &item.name))
                .is_some_and(|approval| approval.fingerprint == fingerprint(item))
    }

    /// The hook to put into the assistants for this item: the current one
    /// once approved, else the version approved before, if any.
    pub fn hook_to_run(&self, item: &LocalItem) -> Option<Hook> {
        let hook = if self.approved(item) {
            hooks::parse_hook(&item.files.get("hook.json")?.bytes).ok()?
        } else {
            self.items
                .get(&item::key(&item.kind, &item.name))?
                .hook
                .clone()?
        };
        Some(Hook {
            name: item.name.clone(),
            ..hook
        })
    }

    /// Hooks and plugins in `items` that wait for approval.
    pub fn waiting<'a>(&self, items: impl IntoIterator<Item = &'a LocalItem>) -> Vec<Waiting> {
        items
            .into_iter()
            .filter(|item| needs_approval(item) && !self.approved(item))
            .map(|item| Waiting {
                kind: item.kind.clone(),
                name: item.name.clone(),
                detail: describe(item),
                changed: self.items.contains_key(&item::key(&item.kind, &item.name)),
            })
            .collect()
    }

    /// Forgets approvals of items the pouch no longer has, so one that
    /// comes back asks again.
    pub fn keep_only<'a>(&mut self, items: impl IntoIterator<Item = &'a LocalItem>) {
        let keys: std::collections::BTreeSet<String> = items
            .into_iter()
            .map(|item| item::key(&item.kind, &item.name))
            .collect();
        self.items.retain(|key, _| keys.contains(key));
    }
}

/// What an item would run, in one line.
pub fn describe(item: &LocalItem) -> String {
    match item.kind.as_str() {
        "hook" => match item
            .files
            .get("hook.json")
            .map(|f| hooks::parse_hook(&f.bytes))
        {
            Some(Ok(hook)) => {
                let on = match hooks_matcher(&hook) {
                    Some(matcher) => format!("{} ({matcher})", hook.event),
                    None => hook.event.clone(),
                };
                format!("{on}: {}", hook.command)
            }
            Some(Err(error)) => error.to_string(),
            None => "no hook.json".into(),
        },
        "plugin" => crate::plugins::describe(item),
        _ => String::new(),
    }
}

fn hooks_matcher(hook: &Hook) -> Option<&str> {
    let matcher = hook.matcher.trim();
    (!matches!(matcher, "" | "*" | ".*")).then_some(matcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::File;

    fn hook(command: &str) -> LocalItem {
        let hook = Hook {
            name: "format".into(),
            description: String::new(),
            event: "PostToolUse".into(),
            matcher: "Edit".into(),
            command: command.into(),
            timeout: None,
        };
        LocalItem {
            kind: "hook".into(),
            name: "format".into(),
            files: [("hook.json".to_string(), File::new(hooks::hook_file(&hook)))].into(),
        }
    }

    #[test]
    fn changed_hooks_wait_and_the_approved_one_keeps_running() {
        let mut approvals = Approvals::default();
        let first = hook("prettier .");
        assert_eq!(approvals.waiting([&first]).len(), 1);
        assert_eq!(approvals.hook_to_run(&first), None);
        approvals.approve(&first);
        assert!(approvals.waiting([&first]).is_empty());

        let changed = hook("rm -rf ~");
        let waiting = approvals.waiting([&changed]);
        assert_eq!(waiting[0].detail, "PostToolUse (Edit): rm -rf ~");
        assert!(waiting[0].changed);
        assert_eq!(
            approvals.hook_to_run(&changed).map(|h| h.command),
            Some("prettier .".into())
        );

        approvals.keep_only([]);
        assert!(approvals.items.is_empty());
    }

    #[test]
    fn other_kinds_never_wait() {
        let skill = LocalItem {
            kind: "skill".into(),
            name: "pdf".into(),
            files: [("SKILL.md".to_string(), File::new("# PDF"))].into(),
        };
        assert!(Approvals::default().approved(&skill));
    }
}
