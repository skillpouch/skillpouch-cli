//! `config.toml`: user settings. Every field is optional in the file.

use crate::home::{read_optional, write_private};
use crate::{CoreError, Home, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const DEFAULT_API_URL: &str = "https://skillpouch.net";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Server origin. The web app and the API share it (`/v1/*`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Name shown on the Devices page; defaults to the hostname.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// `false` keeps SkillPouch from updating itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_update: Option<bool>,
    /// What this computer syncs.
    #[serde(skip_serializing_if = "SyncChoice::is_everything")]
    pub sync: SyncChoice,
}

/// What this computer syncs: everything, except the assistants and kinds
/// turned off here. Other computers choose for themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyncChoice {
    /// Assistant ids (`codex`) this computer leaves alone.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub skip_assistants: BTreeSet<String>,
    /// Kinds (`hook`, `mcp_server`) this computer doesn't put into assistants.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub skip_kinds: BTreeSet<String>,
}

impl SyncChoice {
    pub fn is_everything(&self) -> bool {
        self.skip_assistants.is_empty() && self.skip_kinds.is_empty()
    }

    pub fn assistant_on(&self, id: &str) -> bool {
        !self.skip_assistants.contains(id)
    }

    pub fn kind_on(&self, kind: &str) -> bool {
        !self.skip_kinds.contains(kind)
    }
}

impl Config {
    pub fn load(home: &Home) -> Result<Self> {
        let path = home.config_file();
        let Some(bytes) = read_optional(&path)? else {
            return Ok(Self::default());
        };
        let text = String::from_utf8(bytes).map_err(|_| CoreError::Invalid {
            path: path.clone(),
            message: "not UTF-8".into(),
        })?;
        toml::from_str(&text).map_err(|error| CoreError::Invalid {
            path,
            message: error.message().to_string(),
        })
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        home.ensure_private_dir(home.root())?;
        let path = home.config_file();
        let text = toml::to_string(self).map_err(|error| CoreError::Invalid {
            path: path.clone(),
            message: error.to_string(),
        })?;
        write_private(&path, text.as_bytes())
    }

    /// `$SKILLPOUCH_API_URL`, else the file, else skillpouch.net. No trailing slash.
    pub fn api_url(&self) -> String {
        let from_env = std::env::var("SKILLPOUCH_API_URL")
            .ok()
            .filter(|url| !url.is_empty());
        from_env
            .or_else(|| self.api_url.clone())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string())
            .trim_end_matches('/')
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_default_and_saves_round_trip() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("sp-config-{}", std::process::id()));
        let home = Home::new(&dir);
        assert_eq!(Config::load(&home)?, Config::default());

        let config = Config {
            api_url: Some("http://localhost:5173/".into()),
            device_name: Some("laptop".into()),
            auto_update: Some(false),
            sync: SyncChoice {
                skip_assistants: ["codex".to_string()].into(),
                skip_kinds: ["hook".to_string()].into(),
            },
        };
        config.save(&home)?;
        assert_eq!(Config::load(&home)?, config);
        assert!(!config.sync.assistant_on("codex") && config.sync.kind_on("skill"));
        let text = std::fs::read_to_string(home.config_file()).unwrap_or_default();
        assert!(text.contains("[sync]"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn rejects_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("sp-config-bad-{}", std::process::id()));
        let home = Home::new(&dir);
        let _ = home.ensure_private_dir(home.root());
        let _ = std::fs::write(home.config_file(), "api_ur = \"x\"\n");
        assert!(matches!(
            Config::load(&home),
            Err(CoreError::Invalid { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
