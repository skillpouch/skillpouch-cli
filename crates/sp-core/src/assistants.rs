//! Which AI assistants are installed, judged by their config folders. Only a
//! hint for the Devices page; the harness adapters will do the real work.

use std::path::Path;

/// `(id, name, folders relative to the user's home; any one is enough)`.
const KNOWN: &[(&str, &str, &[&str])] = &[
    ("claude-code", "Claude Code", &[".claude"]),
    ("codex", "Codex", &[".codex"]),
    ("opencode", "OpenCode", &[".config/opencode"]),
    ("gemini-cli", "Gemini CLI", &[".gemini"]),
    ("cursor", "Cursor", &[".cursor"]),
    ("windsurf", "Windsurf", &[".codeium/windsurf"]),
    ("copilot", "GitHub Copilot", &[".copilot"]),
    // Its app data folders and shared settings
    // (<https://antigravity.google/docs/hooks>, <https://antigravity.google/docs/mcp>).
    (
        "antigravity",
        "Antigravity",
        &[
            ".gemini/antigravity",
            ".gemini/antigravity-ide",
            ".gemini/antigravity-cli",
            ".gemini/config",
        ],
    ),
];

/// One assistant SkillPouch knows, and whether this computer has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assistant {
    pub id: &'static str,
    pub name: &'static str,
    /// The folder it was found in, else where it would be (relative to home).
    pub folder: &'static str,
    pub installed: bool,
}

/// Every known assistant, installed ones first.
pub fn scan(user_home: &Path) -> Vec<Assistant> {
    let mut found: Vec<Assistant> = KNOWN
        .iter()
        .map(|(id, name, folders)| {
            let present = folders
                .iter()
                .find(|folder| user_home.join(folder).is_dir());
            Assistant {
                id,
                name,
                folder: present.copied().unwrap_or(folders[0]),
                installed: present.is_some(),
            }
        })
        .collect();
    found.sort_by_key(|assistant| !assistant.installed);
    found
}

/// Ids (as the server expects them) of the assistants found under `user_home`.
pub fn detect(user_home: &Path) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|(_, _, folders)| folders.iter().any(|folder| user_home.join(folder).is_dir()))
        .map(|(id, _, _)| (*id).to_string())
        .collect()
}

/// The display name for a server-side id (`claude-code` → `Claude Code`).
pub fn name_of(id: &str) -> Option<&'static str> {
    KNOWN
        .iter()
        .find(|(known, _, _)| *known == id)
        .map(|(_, name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_assistants_by_their_folders() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("sp-assistants-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".claude"))?;
        std::fs::create_dir_all(dir.join(".config/opencode"))?;
        std::fs::write(dir.join(".codex"), "not a folder")?;
        assert_eq!(detect(&dir), ["claude-code", "opencode"]);
        let scanned = scan(&dir);
        assert_eq!(scanned[0].name, "Claude Code");
        assert!(scanned[1].installed && !scanned[2].installed);
        assert_eq!(scanned.len(), KNOWN.len());
        assert_eq!(name_of("gemini-cli"), Some("Gemini CLI"));
        std::fs::create_dir_all(dir.join(".gemini/antigravity-cli"))?;
        assert_eq!(
            detect(&dir),
            ["claude-code", "opencode", "gemini-cli", "antigravity"]
        );
        let antigravity = scan(&dir).into_iter().find(|a| a.id == "antigravity");
        assert_eq!(
            antigravity.map(|a| (a.folder, a.installed)),
            Some((".gemini/antigravity-cli", true))
        );
        std::fs::remove_dir_all(&dir)
    }
}
