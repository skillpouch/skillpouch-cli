//! Items as plain files: the pouch folder `pouches/<id>/<kind folder>/<name>/…`.

use crate::Result;
use crate::error::io;
use sp_harness::{IGNORED_DIRS, ignored_file};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Kind, its folder in the pouch and its main file, as the web stores them.
pub const KINDS: [(&str, &str, &str); 7] = [
    ("instructions", "instructions", "INSTRUCTIONS.md"),
    ("skill", "skills", "SKILL.md"),
    ("agent", "agents", "AGENT.md"),
    ("command", "commands", "COMMAND.md"),
    ("mcp_server", "mcp-servers", "server.json"),
    ("hook", "hooks", "hook.json"),
    ("plugin", "plugins", "plugin.json"),
];

/// A pouch has one set of instructions, always under this name.
pub const INSTRUCTIONS: &str = "instructions";

/// Files at or above this size aren't synced (the server's blob limit).
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Most files one item may have.
pub const MAX_FILES: usize = 2000;

/// Left behind by an interrupted write; never synced.
const TMP_SUFFIX: &str = ".sp-tmp";

pub fn folder_of(kind: &str) -> Option<&'static str> {
    KINDS.iter().find(|k| k.0 == kind).map(|k| k.1)
}

pub fn main_file(kind: &str) -> Option<&'static str> {
    KINDS.iter().find(|k| k.0 == kind).map(|k| k.2)
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`, the names the web accepts.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A valid name from anything (`git/commit` → `git-commit`), if one can be made.
pub fn safe_name(text: &str) -> Option<String> {
    let replaced: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = replaced.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    let name: String = trimmed.chars().take(64).collect();
    let name = name.trim_end_matches('-').to_string();
    valid_name(&name).then_some(name)
}

/// The key two sides match items by: kind and case-folded name.
pub fn key(kind: &str, name: &str) -> String {
    format!("{kind}/{}", name.to_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub bytes: Vec<u8>,
    pub exec: bool,
}

impl File {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
            exec: false,
        }
    }
}

/// Files by path relative to the item, `/`-separated.
pub type Files = BTreeMap<String, File>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalItem {
    pub kind: String,
    pub name: String,
    pub files: Files,
}

/// A relative path from the server that is safe to write under an item.
pub fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// One pouch's folder.
#[derive(Debug, Clone)]
pub struct PouchDir {
    root: PathBuf,
}

/// What reading the folder left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// `skills/pdf/big.bin`: too large to sync.
    pub too_large: Vec<String>,
    /// Folders whose name isn't a valid item name.
    pub bad_names: Vec<String>,
}

impl PouchDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn item_dir(&self, kind: &str, name: &str) -> Option<PathBuf> {
        folder_of(kind).map(|folder| self.root.join(folder).join(name))
    }

    /// Every item in the folder, by key.
    pub fn read(&self) -> Result<(BTreeMap<String, LocalItem>, Skipped)> {
        let mut items = BTreeMap::new();
        let mut skipped = Skipped::default();
        for (kind, folder, _) in KINDS {
            let dir = self.root.join(folder);
            let listing = match fs::read_dir(&dir) {
                Ok(listing) => listing,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io(&dir)(error)),
            };
            for entry in listing.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !entry.file_type().is_ok_and(|t| t.is_dir()) || name.starts_with('.') {
                    continue;
                }
                if !valid_name(&name) {
                    skipped.bad_names.push(format!("{folder}/{name}"));
                    continue;
                }
                let files = read_files(&entry.path(), &format!("{folder}/{name}"), &mut skipped)?;
                if files.is_empty() {
                    continue;
                }
                items.insert(
                    key(kind, &name),
                    LocalItem {
                        kind: kind.to_string(),
                        name,
                        files,
                    },
                );
            }
        }
        Ok((items, skipped))
    }

    /// Makes the item's folder hold exactly `files`.
    pub fn write_item(&self, kind: &str, name: &str, files: &Files) -> Result<()> {
        let Some(dir) = self.item_dir(kind, name) else {
            return Ok(());
        };
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        let mut unused = Skipped::default();
        let existing = read_files(&dir, "", &mut unused)?;
        for path in existing.keys().filter(|path| !files.contains_key(*path)) {
            let file = dir.join(path);
            fs::remove_file(&file).map_err(io(&file))?;
        }
        for (path, file) in files {
            if existing.get(path) == Some(file) {
                continue;
            }
            write_file(&dir.join(path), file)?;
        }
        remove_empty_dirs(&dir);
        Ok(())
    }

    pub fn remove_item(&self, kind: &str, name: &str) -> Result<()> {
        let Some(dir) = self.item_dir(kind, name) else {
            return Ok(());
        };
        match fs::remove_dir_all(&dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(io(&dir)(error)),
            _ => Ok(()),
        }
    }
}

/// Every file under `dir` (links inside are skipped, never followed).
pub fn read_files(dir: &Path, label: &str, skipped: &mut Skipped) -> Result<Files> {
    let mut files = Files::new();
    let mut pending = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        let listing = match fs::read_dir(&folder) {
            Ok(listing) => listing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io(&folder)(error)),
        };
        for entry in listing.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{prefix}{name}");
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !IGNORED_DIRS.contains(&name.as_str()) {
                    pending.push((entry.path(), format!("{path}/")));
                }
                continue;
            }
            if !kind.is_file() || ignored_file(&name) || name.ends_with(TMP_SUFFIX) {
                continue;
            }
            let meta = entry.metadata().map_err(io(&entry.path()))?;
            if meta.len() >= MAX_FILE_BYTES || files.len() >= MAX_FILES {
                skipped.too_large.push(format!("{label}/{path}"));
                continue;
            }
            let bytes = fs::read(entry.path()).map_err(io(&entry.path()))?;
            files.insert(
                path,
                File {
                    bytes,
                    exec: is_exec(&meta),
                },
            );
        }
    }
    Ok(files)
}

/// Writes atomically (temp file, then rename), with the exec bit when asked.
pub fn write_file(path: &Path, file: &File) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}{TMP_SUFFIX}"));
    fs::write(&tmp, &file.bytes).map_err(io(&tmp))?;
    set_exec(&tmp, file.exec)?;
    fs::rename(&tmp, path).map_err(io(path))
}

fn remove_empty_dirs(dir: &Path) {
    let Ok(listing) = fs::read_dir(dir) else {
        return;
    };
    for entry in listing.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_dirs(&entry.path());
            let _ = fs::remove_dir(entry.path());
        }
    }
}

#[cfg(unix)]
fn is_exec(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_exec(_meta: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn set_exec(path: &Path, exec: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if exec { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io(path))
}

#[cfg(not(unix))]
fn set_exec(_path: &Path, _exec: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_web_rules() {
        assert!(valid_name("pdf-tools.v2"));
        assert!(!valid_name("-pdf"));
        assert!(!valid_name("git/commit"));
        assert!(!valid_name(&"a".repeat(65)));
        assert_eq!(safe_name("git/commit").as_deref(), Some("git-commit"));
        assert_eq!(safe_name("  My Skill!").as_deref(), Some("My-Skill"));
        assert_eq!(safe_name("///"), None);
    }

    #[test]
    fn server_paths_must_stay_inside_the_item() {
        assert!(safe_path("scripts/fill.py"));
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a//b",
            "C:/x",
            "a\\b",
        ] {
            assert!(!safe_path(bad), "{bad}");
        }
    }

    #[test]
    fn writes_and_reads_items() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new("tmp")))?;
        let pouch = PouchDir::new(dir.path());
        let mut files = Files::new();
        files.insert("SKILL.md".into(), File::new("# PDF"));
        files.insert(
            "scripts/run.sh".into(),
            File {
                bytes: b"echo".to_vec(),
                // Windows has no executable bit to keep.
                exec: cfg!(unix),
            },
        );
        pouch.write_item("skill", "pdf", &files)?;
        fs::write(dir.path().join("skills/pdf/.DS_Store"), "x").map_err(io(dir.path()))?;
        fs::create_dir_all(dir.path().join("skills/Bad Name")).map_err(io(dir.path()))?;

        let (items, skipped) = pouch.read()?;
        let item = &items["skill/pdf"];
        assert_eq!(item.files, files);
        assert_eq!(skipped.bad_names, ["skills/Bad Name"]);

        files.remove("scripts/run.sh");
        pouch.write_item("skill", "pdf", &files)?;
        assert!(!dir.path().join("skills/pdf/scripts").exists());
        pouch.remove_item("skill", "pdf")?;
        assert!(pouch.read()?.0.is_empty());
        Ok(())
    }
}
