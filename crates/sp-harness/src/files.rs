//! Walking an assistant's folders: skills, Markdown documents, sizes.

use crate::{Item, Kind};
use std::fs;
use std::path::{Path, PathBuf};

/// Folders never counted as part of a skill.
pub const IGNORED_DIRS: &[&str] = &["node_modules", ".git", "__pycache__", ".venv"];

/// Files that are build output, secrets or editor leftovers.
pub fn ignored_file(name: &str) -> bool {
    matches!(name, ".DS_Store" | "Thumbs.db")
        || name.starts_with(".env")
        || name.starts_with("id_rsa")
        || name.starts_with(".#")
        || name.starts_with("~$")
        || name.ends_with('~')
        || [".pyc", ".log", ".pem", ".key", ".swp"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

/// Tells SkillPouch's own links (into `~/.skillpouch`) from the user's files.
pub(crate) struct Own {
    root: Option<PathBuf>,
}

impl Own {
    pub(crate) fn new(skillpouch: &Path) -> Self {
        Self {
            root: fs::canonicalize(skillpouch).ok(),
        }
    }

    fn is_ours(&self, path: &Path) -> bool {
        let Some(root) = &self.root else {
            return false;
        };
        fs::canonicalize(path).is_ok_and(|real| real.starts_with(root))
    }

    /// The size of a regular file (links followed) that isn't ours.
    pub(crate) fn file(&self, path: &Path) -> Option<u64> {
        let meta = fs::metadata(path).ok()?;
        (meta.is_file() && !self.is_ours(path)).then_some(meta.len())
    }
}

/// The folders under `dir` that hold a `SKILL.md`.
pub(crate) fn skills(dir: &Path, own: &Own) -> Vec<Item> {
    let Ok(listing) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in listing.flatten() {
        let path = entry.path();
        let main = path.join("SKILL.md");
        if !path.is_dir() || own.is_ours(&path) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&main) else {
            continue;
        };
        let folder = entry.file_name().to_string_lossy().into_owned();
        let (files, bytes) = tree_size(&path);
        found.push(Item {
            kind: Kind::Skill,
            name: frontmatter_name(&text).unwrap_or(folder),
            path,
            files,
            bytes,
        });
    }
    found
}

/// Files ending in `.ext` under `dir`, named by their path without the
/// extension (`git/commit` for `git/commit.md`).
pub(crate) fn documents(dir: &Path, ext: &str, nested: bool, kind: Kind, own: &Own) -> Vec<Item> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(listing) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in listing.flatten() {
            let path = entry.path();
            let Ok(kind_of) = entry.file_type() else {
                continue;
            };
            if kind_of.is_dir() {
                if nested && !IGNORED_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()) {
                    pending.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|e| e != ext) {
                continue;
            }
            let Some(bytes) = own.file(&path) else {
                continue;
            };
            let Ok(relative) = path
                .with_extension("")
                .strip_prefix(dir)
                .map(Path::to_path_buf)
            else {
                continue;
            };
            let name = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            found.push(Item {
                kind,
                name,
                path,
                files: 1,
                bytes,
            });
        }
    }
    found
}

/// Files and bytes in a skill folder, without following links inside it.
fn tree_size(dir: &Path) -> (usize, u64) {
    let (mut files, mut bytes) = (0, 0);
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(listing) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in listing.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && !IGNORED_DIRS.contains(&name.as_str()) => {
                    pending.push(entry.path());
                }
                Ok(kind) if kind.is_file() && !ignored_file(&name) => {
                    files += 1;
                    bytes += entry.metadata().map_or(0, |meta| meta.len());
                }
                _ => {}
            }
        }
    }
    (files, bytes)
}

/// The `name:` in a document's YAML front matter.
pub(crate) fn frontmatter_name(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    lines
        .take_while(|line| line.trim() != "---")
        .find_map(|line| line.strip_prefix("name:"))
        .map(|value| value.trim().trim_matches(['"', '\'']).to_string())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_name_from_front_matter() {
        assert_eq!(
            frontmatter_name("---\ndescription: x\nname: \"pdf\"\n---\nname: no"),
            Some("pdf".into())
        );
        assert_eq!(frontmatter_name("# Title\nname: no"), None);
        assert_eq!(frontmatter_name("---\nname:\n---"), None);
    }

    #[test]
    fn leaves_out_secrets_and_leftovers() {
        for name in [
            ".env.local",
            "id_rsa.pub",
            "app.log",
            "notes.md~",
            ".DS_Store",
        ] {
            assert!(ignored_file(name), "{name}");
        }
        for name in ["SKILL.md", "env.md", "script.py"] {
            assert!(!ignored_file(name), "{name}");
        }
    }
}
