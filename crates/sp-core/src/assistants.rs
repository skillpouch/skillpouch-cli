//! Which AI assistants are installed, judged by their config folders. Only a
//! hint for the Devices page; the harness adapters will do the real work.

use std::path::Path;

/// `(id, folders relative to the user's home; any one is enough)`.
const KNOWN: &[(&str, &[&str])] = &[
    ("claude-code", &[".claude"]),
    ("codex", &[".codex"]),
    ("opencode", &[".config/opencode"]),
    ("gemini-cli", &[".gemini"]),
    ("cursor", &[".cursor"]),
    ("windsurf", &[".codeium/windsurf"]),
    ("copilot", &[".copilot"]),
    ("antigravity", &[".antigravity", ".gemini/antigravity"]),
];

/// Ids (as the server expects them) of the assistants found under `user_home`.
pub fn detect(user_home: &Path) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|(_, folders)| folders.iter().any(|folder| user_home.join(folder).is_dir()))
        .map(|(id, _)| (*id).to_string())
        .collect()
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
        std::fs::remove_dir_all(&dir)
    }
}
