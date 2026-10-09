//! Plugins. A pouch holds either a whole plugin folder (with
//! `.claude-plugin/plugin.json`) or a reference in `plugin.json`:
//!
//! ```text
//! { "name": "code-review", "source": "code-review@acme-tools",
//!   "marketplace": "acme/claude-plugins" }        Claude Code, Factory
//! { "name": "wakatime", "npm": "opencode-wakatime" }  OpenCode
//! ```
//!
//! Claude Code and Factory get `enabledPlugins` and `extraKnownMarketplaces`
//! entries; plugin folders are offered through a `skillpouch` marketplace
//! in the pouch's `plugins/` folder. OpenCode gets its `plugin` list. Each
//! assistant installs the plugins itself.

use crate::error::io;
use crate::item::{File, LocalItem};
use crate::{Result, SyncError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sp_harness::PluginFormat;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// The marketplace plugin folders in the pouch are offered through.
pub const MARKETPLACE: &str = "skillpouch";
const MANIFEST: &str = ".claude-plugin/plugin.json";

/// A plugin reference (`plugin.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plugin {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// `plugin@marketplace`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Where the marketplace comes from: `owner/repo`, a git URL, or a
    /// Claude Code source object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<Value>,
    /// An npm package (OpenCode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
}

/// What a plugin item is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A whole plugin: offered through the `skillpouch` marketplace.
    Folder {
        plugin: String,
        description: String,
    },
    Reference(Plugin),
}

pub fn of(item: &LocalItem) -> Option<Kind> {
    if let Some(manifest) = item.files.get(MANIFEST) {
        let value: Value = serde_json::from_slice(&manifest.bytes).unwrap_or_default();
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        return Some(Kind::Folder {
            plugin: text("name").unwrap_or_else(|| item.name.clone()),
            description: text("description").unwrap_or_default(),
        });
    }
    let plugin: Plugin = serde_json::from_slice(&item.files.get("plugin.json")?.bytes).ok()?;
    (plugin.source.is_some() || plugin.npm.is_some()).then_some(Kind::Reference(plugin))
}

pub fn plugin_file(plugin: &Plugin) -> File {
    let mut text = serde_json::to_string_pretty(plugin).unwrap_or_default();
    text.push('\n');
    File::new(text)
}

/// `owner/repo`, a URL or a source object → `{ "source": {…} }`.
fn marketplace_entry(value: &Value) -> Option<Value> {
    let source = match value {
        Value::String(text) if text.contains("://") || text.ends_with(".git") => {
            json!({ "source": "git", "url": text })
        }
        Value::String(text) if text.split('/').count() == 2 => {
            json!({ "source": "github", "repo": text })
        }
        Value::Object(fields) if fields.get("source").is_some_and(Value::is_object) => {
            return Some(value.clone());
        }
        Value::Object(_) => value.clone(),
        _ => return None,
    };
    Some(json!({ "source": source }))
}

/// `name@market`: the plugin and the marketplace (an npm scope's `@` doesn't count).
fn split_id(id: &str) -> Option<(&str, &str)> {
    let at = id.char_indices().skip(1).find(|(_, c)| *c == '@')?.0;
    Some((&id[..at], &id[at + 1..]))
}

/// What SkillPouch did to one config file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginFile {
    #[serde(default)]
    pub enabled: BTreeSet<String>,
    #[serde(default)]
    pub marketplaces: BTreeSet<String>,
    #[serde(default)]
    pub npm: BTreeSet<String>,
    /// Plugins on in the file right after writing (ids or packages).
    #[serde(default)]
    pub seen: BTreeSet<String>,
}

/// The plugins turned on in a config file now.
pub fn turned_on(path: &Path, format: PluginFormat) -> BTreeSet<String> {
    on(path, format).0
}

/// The plugins on in a config file: `plugin@market` ids (Claude) or npm
/// packages (OpenCode).
fn on(path: &Path, format: PluginFormat) -> (BTreeSet<String>, Value) {
    let value: Value = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&sp_harness::strip_jsonc(&text)).ok())
        .unwrap_or_default();
    let ids = match format {
        PluginFormat::Claude => value
            .get("enabledPlugins")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(_, on)| on.as_bool() == Some(true))
            .map(|(id, _)| id.clone())
            .collect(),
        PluginFormat::OpenCode => value
            .get("plugin")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect(),
    };
    (ids, value)
}

/// Writes the pouch's plugin items into one config file.
pub fn write(
    path: &Path,
    format: PluginFormat,
    items: &[&LocalItem],
    marketplace_dir: &Path,
    previous: Option<&PluginFile>,
) -> Result<std::result::Result<PluginFile, String>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io(path)(error)),
    };
    let previous = previous.cloned().unwrap_or_default();
    let mut record = PluginFile::default();
    let updated = match format {
        PluginFormat::Claude => {
            let mut enabled = BTreeMap::new();
            let mut markets = BTreeMap::new();
            for item in items {
                match of(item) {
                    Some(Kind::Folder { plugin, .. }) => {
                        enabled.insert(format!("{plugin}@{MARKETPLACE}"), json!(true));
                        markets.insert(
                            MARKETPLACE.to_string(),
                            json!({ "source": { "source": "directory",
                                "path": marketplace_dir.to_string_lossy() } }),
                        );
                    }
                    Some(Kind::Reference(plugin)) => {
                        let Some(id) = plugin.source else { continue };
                        if let (Some((_, market)), Some(entry)) = (
                            split_id(&id),
                            plugin.marketplace.as_ref().and_then(marketplace_entry),
                        ) {
                            markets.insert(market.to_string(), entry);
                        }
                        enabled.insert(id, json!(true));
                    }
                    None => {}
                }
            }
            record.enabled = enabled.keys().cloned().collect();
            record.marketplaces = markets.keys().cloned().collect();
            if text.trim().is_empty() && enabled.is_empty() {
                Ok(text.clone())
            } else {
                crate::jsonc::set_entries(&text, "enabledPlugins", &enabled, &previous.enabled)
                    .and_then(|text| {
                        crate::jsonc::set_entries(
                            &text,
                            "extraKnownMarketplaces",
                            &markets,
                            &previous.marketplaces,
                        )
                    })
            }
        }
        PluginFormat::OpenCode => {
            let (current, _) = on(path, format);
            let wanted: BTreeSet<String> = items
                .iter()
                .filter_map(|item| match of(item) {
                    Some(Kind::Reference(plugin)) => plugin.npm,
                    _ => None,
                })
                .collect();
            let had_list = fs::read_to_string(path).is_ok_and(|t| t.contains("\"plugin\""));
            let mut list: Vec<String> = current
                .iter()
                .filter(|p| !previous.npm.contains(*p) || wanted.contains(*p))
                .cloned()
                .collect();
            // The user's own entry for the same package stays theirs.
            for package in &wanted {
                if !list.contains(package) {
                    list.push(package.clone());
                    record.npm.insert(package.clone());
                } else if previous.npm.contains(package) {
                    record.npm.insert(package.clone());
                }
            }
            if list.is_empty() && !had_list {
                Ok(text.clone())
            } else if list.is_empty()
                && !previous.npm.is_empty()
                && current.len() == previous.npm.len()
            {
                crate::jsonc::set_value(&text, &["plugin"], None)
            } else {
                crate::jsonc::set_value(&text, &["plugin"], Some(&json!(list)))
            }
        }
    };
    let updated = match updated {
        Ok(updated) => updated,
        Err(reason) => return Ok(Err(format!("{}: {reason}", path.display()))),
    };
    if updated != text {
        crate::item::write_file(path, &File::new(updated.into_bytes()))?;
    }
    record.seen = on(path, format).0;
    Ok(Ok(record))
}

/// The `skillpouch` marketplace for the plugin folders in `plugins_dir`
/// (`plugins/.claude-plugin/marketplace.json`; hidden folders aren't items).
pub fn write_marketplace(plugins_dir: &Path, items: &[&LocalItem]) -> Result<()> {
    let path = plugins_dir.join(".claude-plugin/marketplace.json");
    let plugins: Vec<Value> = items
        .iter()
        .filter_map(|item| match of(item)? {
            Kind::Folder {
                plugin,
                description,
            } => Some(json!({
                "name": plugin,
                "source": format!("./{}", item.name),
                "description": description,
            })),
            Kind::Reference(_) => None,
        })
        .collect();
    if plugins.is_empty() {
        let _ = fs::remove_dir_all(plugins_dir.join(".claude-plugin"));
        return Ok(());
    }
    let manifest = json!({
        "name": MARKETPLACE,
        "owner": { "name": "SkillPouch" },
        "plugins": plugins,
    });
    let mut text = serde_json::to_string_pretty(&manifest)
        .map_err(|error| SyncError::Invalid(error.to_string()))?;
    text.push('\n');
    if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        crate::item::write_file(&path, &File::new(text.into_bytes()))?;
    }
    Ok(())
}

/// Plugins turned on in a config since SkillPouch last wrote it (all of
/// them without a record), as references for the pouch. `known` holds
/// marketplace sources the assistant registered on its own
/// (`~/.claude/plugins/known_marketplaces.json`).
pub fn added(
    path: &Path,
    format: PluginFormat,
    previous: Option<&PluginFile>,
    known: &BTreeMap<String, Value>,
) -> Vec<Plugin> {
    let (ids, value) = on(path, format);
    ids.into_iter()
        .filter(|id| !previous.is_some_and(|file| file.seen.contains(id)))
        .filter_map(|id| match format {
            PluginFormat::Claude => {
                let (plugin, market) = split_id(&id)?;
                if market == MARKETPLACE {
                    return None;
                }
                let marketplace = value
                    .pointer(&format!("/extraKnownMarketplaces/{market}/source"))
                    .or_else(|| known.get(market))
                    .cloned();
                Some(Plugin {
                    name: crate::item::safe_name(plugin)?,
                    source: Some(id.clone()),
                    marketplace,
                    ..Plugin::default()
                })
            }
            PluginFormat::OpenCode => {
                let bare = id.trim_start_matches('@');
                let bare = bare.split('@').next().unwrap_or(bare);
                Some(Plugin {
                    name: crate::item::safe_name(bare.rsplit('/').next().unwrap_or(bare))?,
                    npm: Some(id.clone()),
                    ..Plugin::default()
                })
            }
        })
        .collect()
}

/// Marketplace sources Claude Code registered itself (`/plugin marketplace add`).
pub fn known_marketplaces(claude_dir: &Path) -> BTreeMap<String, Value> {
    let path = claude_dir.join("plugins/known_marketplaces.json");
    let value: Value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, entry)| Some((name.clone(), entry.get("source")?.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Files;

    fn item(name: &str, files: &[(&str, &str)]) -> LocalItem {
        LocalItem {
            kind: "plugin".into(),
            name: name.into(),
            files: files
                .iter()
                .map(|(path, text)| (path.to_string(), File::new(*text)))
                .collect::<Files>(),
        }
    }

    fn read(path: &Path) -> Value {
        let text = fs::read_to_string(path).unwrap_or_default();
        serde_json::from_str(&sp_harness::strip_jsonc(&text)).unwrap_or_default()
    }

    #[test]
    fn claude_gets_references_and_the_pouch_marketplace() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            "{\n  // mine\n  \"enabledPlugins\": { \"mine@x\": true }\n}\n",
        )
        .map_err(io(&path))?;
        let review = item(
            "review",
            &[(
                "plugin.json",
                r#"{"name":"review","source":"review@acme","marketplace":"acme/plugins"}"#,
            )],
        );
        let folder = item(
            "lint",
            &[
                (".claude-plugin/plugin.json", r#"{"name":"lint-tools"}"#),
                ("commands/lint.md", "Lint"),
            ],
        );
        let market = dir.path().join("current/plugins");
        let record = write(
            &path,
            PluginFormat::Claude,
            &[&review, &folder],
            &market,
            None,
        )?
        .map_err(SyncError::Invalid)?;
        let value = read(&path);
        assert_eq!(value["enabledPlugins"]["mine@x"], true);
        assert_eq!(value["enabledPlugins"]["review@acme"], true);
        assert_eq!(value["enabledPlugins"]["lint-tools@skillpouch"], true);
        assert_eq!(
            value["extraKnownMarketplaces"]["acme"]["source"]["repo"],
            "acme/plugins"
        );
        assert_eq!(
            value["extraKnownMarketplaces"]["skillpouch"]["source"]["source"],
            "directory"
        );
        assert!(
            fs::read_to_string(&path)
                .map_err(io(&path))?
                .contains("// mine")
        );
        // Nothing new: the user's own plugin was there before, ours aren't "added".
        assert!(added(&path, PluginFormat::Claude, Some(&record), &BTreeMap::new()).is_empty());

        write(&path, PluginFormat::Claude, &[], &market, Some(&record))?
            .map_err(SyncError::Invalid)?;
        let value = read(&path);
        assert_eq!(value["enabledPlugins"], json!({ "mine@x": true }));
        assert_eq!(value["extraKnownMarketplaces"], json!({}));

        write_marketplace(&market, &[&folder, &review])?;
        let manifest = read(&market.join(".claude-plugin/marketplace.json"));
        assert_eq!(manifest["plugins"][0]["source"], "./lint");
        Ok(())
    }

    #[test]
    fn plugins_found_in_configs_become_references() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"enabledPlugins":{"review@acme":true,"off@acme":false,"x@skillpouch":true}}"#,
        )
        .map_err(io(&path))?;
        let known = BTreeMap::from([(
            "acme".to_string(),
            json!({"source":"github","repo":"acme/p"}),
        )]);
        let found = added(&path, PluginFormat::Claude, None, &known);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "review");
        assert_eq!(
            found[0].marketplace,
            Some(json!({"source":"github","repo":"acme/p"}))
        );

        let oc = dir.path().join("opencode.json");
        fs::write(&oc, r#"{"plugin":["@me/tool@1.2","opencode-wakatime"]}"#).map_err(io(&oc))?;
        let found = added(&oc, PluginFormat::OpenCode, None, &BTreeMap::new());
        let names: Vec<_> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["tool", "opencode-wakatime"]);
        Ok(())
    }

    #[test]
    fn opencode_list_keeps_the_users_entries() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("opencode.json");
        fs::write(&path, "{ \"plugin\": [\"mine\"] }").map_err(io(&path))?;
        let waka = item(
            "waka",
            &[(
                "plugin.json",
                r#"{"name":"waka","npm":"opencode-wakatime"}"#,
            )],
        );
        let record = write(&path, PluginFormat::OpenCode, &[&waka], dir.path(), None)?
            .map_err(SyncError::Invalid)?;
        assert_eq!(read(&path)["plugin"], json!(["mine", "opencode-wakatime"]));
        write(
            &path,
            PluginFormat::OpenCode,
            &[],
            dir.path(),
            Some(&record),
        )?
        .map_err(SyncError::Invalid)?;
        assert_eq!(read(&path)["plugin"], json!(["mine"]));
        Ok(())
    }
}
