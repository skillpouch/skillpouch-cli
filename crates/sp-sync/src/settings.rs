//! Per-device agreement on harness preferences. These are data, never files
//! copied from the server: only allowlisted paths and keys can be applied.

use crate::error::io;
use crate::item::{File, Files, LocalItem, PouchDir};
use crate::project::{back_up, backup_path};
use crate::{Result, SyncError, jsonc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sp_core::Home;
use sp_core::home::{read_optional, write_private};
use sp_harness::{SettingsFile, Target};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

type Entries = BTreeMap<String, Value>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceState {
    pub harnesses: BTreeMap<String, HarnessState>,
    /// Signed deletes stop automatic recapture until a new item is received.
    pub deleted: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessState {
    pub baseline: Entries,
    pub excluded: BTreeSet<String>,
    /// This exclusion followed an observed local deletion, not a CLI choice.
    pub absent: BTreeSet<String>,
    /// When the assistant's folder was made (nanoseconds since the epoch),
    /// as of the last agreement: another time means it was installed again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_created: Option<u128>,
}

/// When `path` was made, where the system records it. Folders (unlike
/// files, which editors replace on every save) are only made again when the
/// assistant is installed again.
fn created(path: &Path) -> Option<u128> {
    fs::metadata(path)
        .and_then(|meta| meta.created())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
}

impl DeviceState {
    fn path(home: &Home, pouch: &str) -> PathBuf {
        home.state_dir()
            .join("settings")
            .join(format!("{pouch}.json"))
    }

    pub fn load(home: &Home, pouch: &str) -> Result<Self> {
        let path = Self::path(home, pouch);
        match read_optional(&path)? {
            None => Ok(Self::default()),
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| SyncError::Invalid(format!("invalid settings state: {e}"))),
        }
    }

    pub fn save(&self, home: &Home, pouch: &str) -> Result<()> {
        let path = Self::path(home, pouch);
        if let Some(parent) = path.parent() {
            home.ensure_private_dir(parent)?;
        }
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|e| SyncError::Invalid(e.to_string()))?;
        write_private(&path, &bytes)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<u8>,
    pub files: BTreeMap<String, BTreeMap<String, Value>>,
}

fn invalid(message: impl Into<String>) -> SyncError {
    SyncError::Invalid(message.into())
}

fn ordered(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), ordered(v)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(ordered).collect()),
        _ => value.clone(),
    }
}

impl Setting {
    pub fn from_item(item: &LocalItem) -> Result<Self> {
        if item.kind != "setting"
            || sp_harness::settings(&item.name).is_empty()
            || item.files.len() != 1
        {
            return Err(invalid("invalid settings item"));
        }
        let bytes = &item
            .files
            .get("setting.json")
            .ok_or_else(|| invalid("missing setting.json"))?
            .bytes;
        let setting: Self = serde_json::from_slice(bytes)
            .map_err(|e| invalid(format!("invalid setting.json: {e}")))?;
        if setting.harness != item.name
            || (item.name == "opencode" && !matches!(setting.format, Some(1 | 2)))
            || (item.name != "opencode" && setting.format.is_some())
        {
            return Err(invalid("settings harness or format mismatch"));
        }
        let specs = specs(&item.name, setting.format);
        for (file, values) in &setting.files {
            let Some(spec) = specs.iter().find(|spec| spec.file == file) else {
                return Err(invalid("settings contain an unknown file"));
            };
            for (key, value) in values {
                let valid = if key == "security" && item.name == "gemini-cli" {
                    value
                        .get("auth")
                        .and_then(Value::as_object)
                        .is_some_and(|auth| auth.len() == 1 && auth.contains_key("selectedType"))
                        && value.as_object().is_some_and(|map| map.len() == 1)
                } else if key == "security" && item.name == "qwen-code" {
                    value
                        .as_object()
                        .is_some_and(|map| map.len() == 1 && map.contains_key("auth"))
                } else if key == "options" && item.name == "crush" {
                    value
                        .as_object()
                        .is_some_and(|map| !map.contains_key("data_directory"))
                } else if key == "sessionDefaultSettings" && item.name == "factory" {
                    value.as_object().is_some_and(|map| {
                        map.keys().all(|key| {
                            [
                                "interactionMode",
                                "autonomyLevel",
                                "specModeModel",
                                "specModeReasoningEffort",
                            ]
                            .contains(&key.as_str())
                        })
                    })
                } else {
                    allowed(&item.name, spec, key)
                };
                if !valid
                    || (file == ".env"
                        && !value.as_str().is_some_and(|s| !s.contains(['\n', '\r'])))
                {
                    return Err(invalid("settings contain an unapproved key or value"));
                }
            }
        }
        Ok(setting)
    }

    pub fn item(&self) -> Result<LocalItem> {
        let mut files = Files::new();
        let mut normalized = self.clone();
        for values in normalized.files.values_mut() {
            for value in values.values_mut() {
                *value = ordered(value);
            }
        }
        let mut bytes =
            serde_json::to_vec_pretty(&normalized).map_err(|e| invalid(e.to_string()))?;
        bytes.push(b'\n');
        files.insert("setting.json".into(), File::new(bytes));
        Ok(LocalItem {
            kind: "setting".into(),
            name: self.harness.clone(),
            files,
        })
    }

    pub fn entries(&self) -> Entries {
        self.files
            .iter()
            .flat_map(|(file, keys)| {
                if file == ".env" {
                    return keys
                        .iter()
                        .map(|(key, value)| (format!("{file}:{key}"), value.clone()))
                        .collect::<Vec<_>>()
                        .into_iter();
                }
                let spec = specs(&self.harness, self.format)
                    .iter()
                    .find(|s| s.file == file);
                let text = if file.ends_with(".toml") {
                    toml::to_string(keys).ok()
                } else {
                    serde_json::to_string(keys).ok()
                };
                let values = spec
                    .and_then(|spec| read_entries(&text?, &self.harness, spec))
                    .unwrap_or_default();
                values
                    .into_iter()
                    .map(move |(key, value)| (format!("{file}:{key}"), value))
                    .collect::<Vec<_>>()
                    .into_iter()
            })
            .collect()
    }

    fn set(&mut self, entry: &str, value: Value) {
        if let Some((file, key)) = entry.split_once(':') {
            let keys = self.files.entry(file.into()).or_default();
            if file != ".env" && key.starts_with("security.auth.") {
                let security = keys
                    .entry("security".into())
                    .or_insert_with(|| Value::Object(Default::default()));
                if let Some(root) = security.as_object_mut() {
                    let auth = root
                        .entry("auth")
                        .or_insert_with(|| Value::Object(Default::default()));
                    if let Some(fields) = auth.as_object_mut() {
                        fields.insert(key["security.auth.".len()..].into(), value);
                    }
                }
            } else if file != ".env" && key == "security.auth" {
                keys.insert("security".into(), serde_json::json!({"auth": value}));
            } else if let Some((parent, child)) = key.split_once('.')
                && specs(&self.harness, self.format)
                    .iter()
                    .any(|spec| spec.file == file && spec.maps.contains(&parent))
            {
                let map = keys
                    .entry(parent.into())
                    .or_insert_with(|| Value::Object(Default::default()));
                if let Some(fields) = map.as_object_mut() {
                    fields.insert(child.into(), value);
                }
            } else {
                keys.insert(key.into(), value);
            }
        }
    }
}

fn allowed(id: &str, spec: &SettingsFile, key: &str) -> bool {
    if spec.file == ".env" {
        return !key.is_empty()
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !key.as_bytes()[0].is_ascii_digit();
    }
    let top = key.split('.').next().unwrap_or_default();
    let nested = key == top
        || spec.maps.contains(&top)
            && key.strip_prefix(top).is_some_and(|rest| {
                rest.starts_with('.') && rest.len() > 1 && !rest[1..].contains(':')
            })
        || matches!(key, "security.auth.selectedType" | "security.auth");
    let forbidden = id == "crush" && key.starts_with("options.data_directory");
    spec.keys.contains(&top) && nested && !forbidden
}

/// The files and keys for this format: OpenCode 1 and 2 name them apart.
fn specs(id: &str, format: Option<u8>) -> &'static [SettingsFile] {
    if id == "opencode" && format == Some(1) {
        &sp_harness::OPENCODE_V1_SETTINGS
    } else {
        sp_harness::settings(id)
    }
}

pub fn detect_opencode(text: &str) -> u8 {
    let Ok(value) = serde_json::from_str::<Value>(&sp_harness::strip_jsonc(text)) else {
        return 2;
    };
    if value
        .get("$schema")
        .and_then(Value::as_str)
        .is_some_and(|s| s.contains("/v2/"))
        || ["providers", "plugins", "permissions", "agents"]
            .iter()
            .any(|key| value.get(key).is_some())
    {
        2
    } else if ["provider", "plugin", "permission", "agent"]
        .iter()
        .any(|key| value.get(key).is_some())
    {
        1
    } else {
        2
    }
}

pub(crate) fn local_format(root: &Path) -> u8 {
    let text = ["opencode.json", "opencode.jsonc"]
        .iter()
        .find_map(|file| fs::read_to_string(root.join(file)).ok());
    if let Some(text) = &text {
        let parsed = serde_json::from_str::<Value>(&sp_harness::strip_jsonc(text)).ok();
        let recognized = parsed.as_ref().is_some_and(|value| {
            value
                .get("$schema")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("/v2/"))
                || [
                    "providers",
                    "plugins",
                    "permissions",
                    "agents",
                    "provider",
                    "plugin",
                    "permission",
                    "agent",
                ]
                .iter()
                .any(|key| value.get(key).is_some())
        });
        if recognized {
            return detect_opencode(text);
        }
    }
    // Only OpenCode 2 writes `cli.json` (migrating `tui.json` into it).
    if root.join("cli.json").exists() {
        return 2;
    }
    let version = std::process::Command::new("opencode")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    version.as_deref().map_or(2, version_format)
}

fn version_format(version: &str) -> u8 {
    if version
        .split(|c: char| !c.is_ascii_digit())
        .find(|part| !part.is_empty())
        == Some("1")
    {
        1
    } else {
        2
    }
}

/// Every file settings sync reads for an assistant, in both OpenCode
/// formats, so the background sync notices edits to any of them.
pub fn files(target: &Target) -> Vec<PathBuf> {
    let mut names: BTreeSet<&str> = sp_harness::settings(target.id)
        .iter()
        .map(|spec| spec.file)
        .collect();
    if target.id == "opencode" {
        names.extend(
            sp_harness::OPENCODE_V1_SETTINGS
                .iter()
                .map(|spec| spec.file),
        );
        names.insert("opencode.jsonc");
    }
    names
        .into_iter()
        .map(|name| target.root.join(name))
        .collect()
}

pub fn translated_on(id: &str, root: &Path, setting: &Setting, entry: &str) -> bool {
    id == "opencode"
        && setting.format != Some(local_format(root))
        && translate(&setting.entries(), setting.format, Some(local_format(root)))
            .contains_key(entry)
}

pub fn local_entries(id: &str, root: &Path, setting: &Setting) -> BTreeSet<String> {
    if id == "opencode" && setting.format != Some(local_format(root)) {
        translate(&setting.entries(), setting.format, Some(local_format(root)))
            .into_keys()
            .collect()
    } else {
        BTreeSet::new()
    }
}

fn file_path(root: &Path, id: &str, name: &str) -> PathBuf {
    if id == "opencode"
        && name == "opencode.json"
        && root.join("opencode.jsonc").exists()
        && !root.join(name).exists()
    {
        root.join("opencode.jsonc")
    } else {
        root.join(name)
    }
}

fn env_entries(text: &str) -> Entries {
    let mut result = Entries::new();
    for line in text.lines() {
        let line = line
            .trim_start()
            .strip_prefix("export ")
            .unwrap_or(line.trim_start());
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if key.is_empty()
                || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || key.as_bytes()[0].is_ascii_digit()
            {
                continue;
            }
            let value = value.trim();
            let value = if value.starts_with(['"', '\'']) {
                value
                    .get(1..)
                    .and_then(|tail| tail.find(&value[..1]).map(|end| &tail[..end]))
                    .unwrap_or(value)
            } else {
                value
                    .split_once(" #")
                    .map_or(value, |(value, _)| value.trim_end())
            };
            result.insert(key.into(), Value::String(value.into()));
        }
    }
    result
}

fn read_entries(text: &str, id: &str, spec: &SettingsFile) -> Option<Entries> {
    if spec.file == ".env" {
        return Some(env_entries(text));
    }
    let parsed: Value = if spec.file.ends_with(".toml") {
        serde_json::to_value(toml::from_str::<toml::Value>(text).ok()?).ok()?
    } else {
        serde_json::from_str(&sp_harness::strip_jsonc(text)).ok()?
    };
    parsed.as_object()?;
    let mut entries = Entries::new();
    for key in spec.keys {
        let path: Vec<&str> = key.split('.').collect();
        let value = path.iter().try_fold(&parsed, |v, p| v.get(*p));
        let Some(value) = value else {
            continue;
        };
        if spec.maps.contains(key) {
            if let Some(map) = value.as_object() {
                for (name, value) in map {
                    entries.insert(format!("{key}.{name}"), ordered(value));
                }
            }
        } else if id == "crush" && *key == "options" {
            if let Some(map) = value.as_object() {
                let mut map = map.clone();
                map.remove("data_directory");
                if !map.is_empty() {
                    entries.insert(key.to_string(), ordered(&Value::Object(map)));
                }
            }
        } else if id == "factory" && *key == "sessionDefaultSettings" {
            if let Some(map) = value.as_object() {
                let filtered: serde_json::Map<String, Value> = map
                    .iter()
                    .filter(|(key, _)| {
                        [
                            "interactionMode",
                            "autonomyLevel",
                            "specModeModel",
                            "specModeReasoningEffort",
                        ]
                        .contains(&key.as_str())
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                if !filtered.is_empty() {
                    entries.insert(key.to_string(), ordered(&Value::Object(filtered)));
                }
            }
        } else {
            entries.insert(key.to_string(), ordered(value));
        }
    }
    Some(entries)
}

fn write_env(text: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut found = false;
    for line in &mut lines {
        let trimmed = line.trim_start();
        let raw = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some((name, old)) = raw.split_once('=')
            && name.trim() == key
        {
            let prefix_len = line.len() - old.len();
            let old = old.trim();
            let quote = if old.starts_with('\'') {
                "'"
            } else if old.starts_with('"') {
                "\""
            } else {
                ""
            };
            let suffix = if quote.is_empty() {
                old.find(" #").map_or("", |n| &old[n..])
            } else {
                old.get(1..)
                    .and_then(|rest| rest.find(quote).map(|n| &rest[n + quote.len()..]))
                    .unwrap_or("")
            };
            line.replace_range(prefix_len.., &format!("{quote}{value}{quote}{suffix}"));
            found = true;
            break;
        }
    }
    if !found {
        lines.push(format!("{key}={value}"));
    }
    format!("{}\n", lines.join("\n"))
}

fn toml_value(value: &Value) -> Option<toml_edit::Value> {
    Some(match value {
        Value::String(s) => s.as_str().into(),
        Value::Bool(v) => (*v).into(),
        Value::Number(n) if n.is_i64() => n.as_i64()?.into(),
        Value::Number(n) => n.as_f64()?.into(),
        Value::Array(values) => toml_edit::Value::Array(
            values
                .iter()
                .map(toml_value)
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .collect(),
        ),
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, value) in map {
                table.insert(key, toml_value(value)?);
            }
            toml_edit::Value::InlineTable(table)
        }
        Value::Null => return None,
    })
}

fn set_toml(
    doc: &mut toml_edit::DocumentMut,
    path: &[&str],
    value: &Value,
) -> std::result::Result<(), String> {
    use toml_edit::{Item, Table};
    let mut table = doc.as_table_mut();
    for key in &path[..path.len() - 1] {
        if !table.contains_key(key) {
            table.insert(key, Item::Table(Table::new()));
        }
        table = table
            .get_mut(key)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| format!("{key} isn't a table"))?;
    }
    let last = path[path.len() - 1];
    if let Some(fields) = value.as_object() {
        if !table.contains_key(last) {
            table.insert(last, Item::Table(Table::new()));
        }
        let dest = table
            .get_mut(last)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| format!("{last} isn't a table"))?;
        for (key, value) in fields {
            if let Some(mut wanted) = toml_value(value) {
                if let Some(previous) = dest.get_mut(key).and_then(Item::as_value_mut) {
                    *wanted.decor_mut() = previous.decor().clone();
                    *previous = wanted;
                } else {
                    dest.insert(key, Item::Value(wanted));
                }
            }
        }
    } else {
        let mut wanted = toml_value(value).ok_or("invalid TOML value")?;
        if let Some(previous) = table.get(last).and_then(Item::as_value) {
            *wanted.decor_mut() = previous.decor().clone();
        }
        if let Some(previous) = table.get_mut(last).and_then(Item::as_value_mut) {
            *previous = wanted;
        } else {
            table.insert(last, Item::Value(wanted));
        }
    }
    Ok(())
}

fn write_entry(
    text: &str,
    file: &str,
    entry: &str,
    value: &Value,
) -> std::result::Result<String, String> {
    if file == ".env" {
        return value
            .as_str()
            .map(|v| write_env(text, entry, v))
            .ok_or("invalid .env value".into());
    }
    if entry == "sessionDefaultSettings"
        && file == "settings.json"
        && let Some(settings) = value.as_object()
    {
        let mut text = text.to_string();
        for (name, value) in settings {
            text = jsonc::set_value(&text, &[entry, name], Some(value))?;
        }
        return Ok(text);
    }
    let path: Vec<&str> = if let Some(field) = entry.strip_prefix("security.auth.") {
        vec!["security", "auth", field]
    } else if let Some((top, child)) = entry.split_once('.') {
        vec![top, child]
    } else {
        vec![entry]
    };
    if file.ends_with(".toml") {
        let mut doc: toml_edit::DocumentMut =
            text.parse().map_err(|e| format!("invalid TOML: {e}"))?;
        set_toml(&mut doc, &path, value)?;
        Ok(doc.to_string())
    } else {
        jsonc::set_value(text, &path, Some(value))
    }
}

/// Read a file only when it exists and parses. Missing and broken files cannot
/// establish that the user deleted any entry.
fn read_local(root: &Path, id: &str, format: Option<u8>) -> (Entries, BTreeSet<String>) {
    let mut entries = Entries::new();
    let mut readable = BTreeSet::new();
    for spec in specs(id, format) {
        let path = file_path(root, id, spec.file);
        if let Ok(text) = fs::read_to_string(path)
            && let Some(values) = read_entries(&text, id, spec)
        {
            readable.insert(spec.file.into());
            entries.extend(
                values
                    .into_iter()
                    .map(|(key, value)| (format!("{}:{key}", spec.file), value)),
            );
        }
    }
    (entries, readable)
}

fn file_of(entry: &str) -> &str {
    entry.split_once(':').map_or("", |(file, _)| file)
}

fn merge(
    pouch: &mut Entries,
    local: &Entries,
    readable: &BTreeSet<String>,
    state: &mut HarnessState,
) -> Entries {
    let mut writes = Entries::new();
    for (key, value) in local {
        if state.excluded.contains(key) {
            if !state.absent.contains(key) && state.baseline.get(key) == Some(value) {
                continue;
            }
            state.excluded.remove(key);
            state.absent.remove(key);
        }
        let remote = pouch.get(key).filter(|remote| *remote != value).cloned();
        match (state.baseline.get(key), remote) {
            // Changed elsewhere, or first meeting (a new computer, a fresh
            // install): the pouch wins over whatever default was here.
            (agreed, Some(remote)) if agreed.is_none_or(|agreed| agreed == value) => {
                writes.insert(key.clone(), remote.clone());
                state.baseline.insert(key.clone(), remote);
            }
            (Some(agreed), _) if agreed == value => {}
            _ => {
                // Edited here since the last agreement: the last writer wins.
                pouch.insert(key.clone(), value.clone());
                state.baseline.insert(key.clone(), value.clone());
            }
        }
    }
    let keys: BTreeSet<String> = pouch.keys().chain(state.baseline.keys()).cloned().collect();
    for key in keys {
        if local.contains_key(&key) || !readable.contains(file_of(&key)) {
            continue;
        }
        if state.baseline.contains_key(&key) {
            state.absent.insert(key.clone());
            state.excluded.insert(key);
        } else if !state.excluded.contains(&key)
            && let Some(value) = pouch.get(&key)
        {
            writes.insert(key.clone(), value.clone());
            state.baseline.insert(key, value.clone());
        }
    }
    writes
}

fn apply(root: &Path, id: &str, entries: &Entries, backups: &Path) -> Result<Vec<PathBuf>> {
    let mut backed_up = Vec::new();
    let mut staged: BTreeMap<String, String> = BTreeMap::new();
    for (entry, value) in entries {
        let Some((file, key)) = entry.split_once(':') else {
            continue;
        };
        let path = file_path(root, id, file);
        let text = if let Some(text) = staged.get(file) {
            text.clone()
        } else {
            fs::read_to_string(&path).unwrap_or_default()
        };
        let mut value = value.clone();
        if id == "crush"
            && key == "options"
            && let Ok(current) = serde_json::from_str::<Value>(&sp_harness::strip_jsonc(&text))
            && let Some(directory) = current.get("options").and_then(|v| v.get("data_directory"))
            && let Some(options) = value.as_object_mut()
        {
            options.insert("data_directory".into(), directory.clone());
        }
        let next = write_entry(&text, file, key, &value)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        staged.insert(file.into(), next);
    }
    for (file, text) in staged {
        let path = file_path(root, id, &file);
        if fs::read_to_string(&path).ok().as_deref() == Some(&text) {
            continue;
        }
        if fs::symlink_metadata(&path).is_ok() {
            if backup_path(&path, backups).exists() {
                let aside = backup_path(&path, backups).with_extension("setting-backup");
                if aside.exists() {
                    return Err(invalid(format!(
                        "backup already exists: {}",
                        aside.display()
                    )));
                }
                fs::rename(&path, &aside).map_err(io(&path))?;
                backed_up.push(aside);
            } else {
                backed_up.push(back_up(&path, backups)?);
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        write_private(&path, text.as_bytes())?;
    }
    Ok(backed_up)
}

/// Restore one selected value now, so the next capture cannot mistake the
/// stale local value for a new edit and overwrite the pouch copy.
pub fn include(
    root: &Path,
    id: &str,
    entry: &str,
    item: &LocalItem,
    backups: &Path,
    state: &mut HarnessState,
) -> Result<()> {
    let setting = Setting::from_item(item)?;
    let format = (id == "opencode").then(|| local_format(root));
    let entries = if setting.format == format {
        setting.entries()
    } else {
        translate(&setting.entries(), setting.format, format)
    };
    let value = entries
        .get(entry)
        .ok_or_else(|| invalid("entry is not available for this format"))?;
    apply(
        root,
        id,
        &Entries::from([(entry.into(), value.clone())]),
        backups,
    )?;
    state.excluded.remove(entry);
    state.absent.remove(entry);
    state.baseline.insert(entry.into(), value.clone());
    Ok(())
}

pub fn exclude(root: &Path, id: &str, entry: &str, state: &mut HarnessState) {
    let format = (id == "opencode").then(|| local_format(root));
    let (local, _) = read_local(root, id, format);
    if let Some(value) = local.get(entry) {
        state.baseline.insert(entry.into(), value.clone());
        state.absent.remove(entry);
    } else {
        state.absent.insert(entry.into());
    }
    state.excluded.insert(entry.into());
}

/// Before engine sync, capture local changes in the pouch. Only already
/// present items are edited; the engine sees the exact same normal item type.
pub fn capture(dir: &PouchDir, targets: &[Target], state: &mut DeviceState) -> Result<()> {
    let (items, _) = dir.read()?;
    for target in targets {
        if sp_harness::settings(target.id).is_empty() || target.read_only.is_some() {
            continue;
        }
        let format = (target.id == "opencode").then(|| local_format(&target.root));
        let (local, readable) = read_local(&target.root, target.id, format);
        let Some(previous) = items.get(&format!("setting/{}", target.id)) else {
            if state.deleted.contains(target.id) {
                continue;
            }
            if !local.is_empty() {
                let mut setting = Setting {
                    harness: target.id.into(),
                    format,
                    files: BTreeMap::new(),
                };
                for (key, value) in &local {
                    setting.set(key, value.clone());
                }
                let item = setting.item()?;
                dir.write_item("setting", target.id, &item.files)?;
                // No agreement until the server has resolved this item. An
                // existing remote item can still arrive in this first sync.
            }
            continue;
        };
        let mut setting = Setting::from_item(previous)?;
        let pouch = if setting.format != format {
            translate(&setting.entries(), setting.format, format)
        } else {
            setting.entries()
        };
        let device = state.harnesses.entry(target.id.into()).or_default();
        // Installed again between two syncs: start over, like a new computer.
        if let (Some(before), Some(now)) = (device.folder_created, created(&target.root))
            && before != now
        {
            *device = HarnessState::default();
        }
        for file in readable {
            let keys: Vec<_> = device
                .baseline
                .keys()
                .filter(|key| file_of(key) == file)
                .cloned()
                .collect();
            if keys.len() >= 2 && keys.iter().all(|key| !local.contains_key(key)) {
                for key in keys {
                    device.baseline.remove(&key);
                    device.excluded.remove(&key);
                }
            }
        }
        let mut edits = Entries::new();
        for (key, value) in &local {
            if device.excluded.contains(key) {
                if !device.absent.contains(key) && device.baseline.get(key) == Some(value) {
                    continue;
                }
                device.excluded.remove(key);
                device.absent.remove(key);
            }
            match device.baseline.get(key) {
                // An unobserved pouch entry must not be overwritten by first
                // capture: projection brings it here instead.
                None if pouch.contains_key(key) => {}
                // Agreement happens after verified server sync. If the
                // server advanced too, projection must still see L != B.
                Some(agreed) if agreed == value => {}
                _ => {
                    edits.insert(key.clone(), value.clone());
                }
            }
        }
        if !edits.is_empty() {
            put(&mut setting, &edits, format);
            dir.write_item("setting", target.id, &setting.item()?.files)?;
        }
    }
    Ok(())
}

/// Puts this computer's edits into the item, which keeps its own format:
/// rewriting it in another OpenCode version's names would lose what only
/// that version has. An edit with no counterpart there stays local.
fn put(setting: &mut Setting, edits: &Entries, format: Option<u8>) {
    for (key, value) in translate(edits, format, setting.format) {
        setting.set(&key, value);
    }
}

/// After verified engine sync, project preferences. No pouch item means no
/// changes to the user's files (including when a signed delete removed it).
pub fn project(
    dir: &PouchDir,
    items: &BTreeMap<String, LocalItem>,
    targets: &[Target],
    state: &mut DeviceState,
    backups: &Path,
) -> Result<Vec<PathBuf>> {
    let mut backed_up = Vec::new();
    let installed: BTreeSet<_> = targets.iter().map(|target| target.id).collect();
    state
        .harnesses
        .retain(|id, _| installed.contains(id.as_str()));
    for target in targets {
        if sp_harness::settings(target.id).is_empty() || target.read_only.is_some() {
            continue;
        }
        let Some(item) = items.get(&format!("setting/{}", target.id)) else {
            continue;
        };
        let setting = Setting::from_item(item)?;
        state.deleted.remove(target.id);
        let format = (target.id == "opencode").then(|| local_format(&target.root));
        let (local, readable) = read_local(&target.root, target.id, format);
        let state = state.harnesses.entry(target.id.into()).or_default();
        let mut pouch = setting.entries();
        if setting.format != format {
            pouch = translate(&pouch, setting.format, format);
        }
        let before = pouch.clone();
        let mut writes = merge(&mut pouch, &local, &readable, state);
        let edits: Entries = pouch
            .iter()
            .filter(|(key, value)| before.get(*key) != Some(*value))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if !edits.is_empty() {
            let mut updated = setting.clone();
            put(&mut updated, &edits, format);
            dir.write_item("setting", target.id, &updated.item()?.files)?;
        }
        // Missing files on an installed harness receive the pouch's values;
        // they do not count as deletions or exclusions.
        for (key, value) in &pouch {
            if !readable.contains(file_of(key)) && !state.excluded.contains(key) {
                writes.insert(key.clone(), value.clone());
                state.baseline.insert(key.clone(), value.clone());
            }
        }
        backed_up.extend(apply(&target.root, target.id, &writes, backups)?);
        state.folder_created = created(&target.root);
    }
    Ok(backed_up)
}

/// OpenCode 1 ↔ 2. Only what both versions mean the same way crosses over;
/// keybinds keep their own names per version and stay with it.
fn translate(entries: &Entries, source: Option<u8>, dest: Option<u8>) -> Entries {
    let up = match (source, dest) {
        (Some(1), Some(2)) => true,
        (Some(2), Some(1)) => false,
        _ => return entries.clone(),
    };
    let mut out = Entries::new();
    let mut scroll = serde_json::Map::new();
    for (key, value) in entries {
        let Some((file, name)) = key.split_once(':') else {
            continue;
        };
        let (top, rest) = name
            .split_once('.')
            .map_or((name, None), |(t, r)| (t, Some(r)));
        match file {
            "opencode.json" => {
                let renamed = match (top, up) {
                    ("provider", true) => "providers",
                    ("providers", false) => "provider",
                    ("agent", true) => "agents",
                    ("agents", false) => "agent",
                    ("permission", true) => "permissions",
                    ("permissions", false) => "permission",
                    (
                        "model" | "share" | "compaction" | "username" | "default_agent"
                        | "tool_output",
                        _,
                    ) => top,
                    _ => continue,
                };
                let value = if renamed.starts_with("provider") && rest.is_some() {
                    provider(value, up)
                } else {
                    value.clone()
                };
                let name = rest.map_or(renamed.to_string(), |rest| format!("{renamed}.{rest}"));
                out.insert(format!("opencode.json:{name}"), value);
            }
            "tui.json" if up => match name {
                "theme" => {
                    out.insert(
                        "cli.json:theme".into(),
                        serde_json::json!({ "name": value }),
                    );
                }
                "mouse" | "cursor" | "attention" => {
                    out.insert(format!("cli.json:{name}"), value.clone());
                }
                "leader_timeout" => {
                    out.insert(
                        "cli.json:leader".into(),
                        serde_json::json!({ "timeout": value }),
                    );
                }
                "scroll_speed" => {
                    scroll.insert("speed".into(), value.clone());
                }
                "scroll_acceleration" => {
                    if let Some(enabled) = value.get("enabled") {
                        scroll.insert("acceleration".into(), enabled.clone());
                    }
                }
                _ => {}
            },
            "cli.json" if !up => {
                let mut put = |name: &str, value: Option<&Value>| {
                    if let Some(value) = value {
                        out.insert(format!("tui.json:{name}"), value.clone());
                    }
                };
                match name {
                    "theme" => put("theme", value.get("name")),
                    "mouse" | "cursor" | "attention" => put(name, Some(value)),
                    "leader" => put("leader_timeout", value.get("timeout")),
                    "scroll" => {
                        put("scroll_speed", value.get("speed"));
                        let enabled = value
                            .get("acceleration")
                            .map(|on| serde_json::json!({ "enabled": on }));
                        put("scroll_acceleration", enabled.as_ref());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if !scroll.is_empty() {
        out.insert("cli.json:scroll".into(), Value::Object(scroll));
    }
    out
}

/// A custom provider: OpenCode 1's `npm`/`options` and models' `id`/`options`
/// are OpenCode 2's `package` (`aisdk:` for AI SDK packages)/`settings`
/// (headers apart) and `modelID`/`settings`.
fn provider(value: &Value, up: bool) -> Value {
    let mut value = value.clone();
    let Some(map) = value.as_object_mut() else {
        return value;
    };
    let rename = |map: &mut serde_json::Map<String, Value>, from: &str, to: &str| {
        if let Some(moved) = map.remove(from) {
            map.insert(to.into(), moved);
        }
    };
    if up {
        if let Some(Value::String(npm)) = map.remove("npm") {
            let package = if npm.starts_with("aisdk:") || npm.starts_with("@opencode/") {
                npm
            } else {
                format!("aisdk:{npm}")
            };
            map.insert("package".into(), Value::String(package));
        }
        if let Some(mut options) = map.remove("options") {
            if let Some(fields) = options.as_object_mut()
                && let Some(headers) = fields.remove("headers")
            {
                map.insert("headers".into(), headers);
            }
            map.insert("settings".into(), options);
        }
        if let Some(api) = map.remove("api") {
            let settings = map
                .entry("settings")
                .or_insert_with(|| Value::Object(Default::default()));
            if let Some(settings) = settings.as_object_mut() {
                settings.entry("baseURL").or_insert(api);
            }
        }
    } else {
        if let Some(Value::String(package)) = map.remove("package") {
            let npm = package
                .strip_prefix("aisdk:")
                .unwrap_or(&package)
                .to_string();
            map.insert("npm".into(), Value::String(npm));
        }
        if let Some(mut settings) = map.remove("settings") {
            if let Some(options) = settings.as_object_mut()
                && let Some(headers) = map.remove("headers")
            {
                options.insert("headers".into(), headers);
            }
            map.insert("options".into(), settings);
        }
    }
    if let Some(models) = map.get_mut("models").and_then(Value::as_object_mut) {
        for model in models.values_mut().filter_map(Value::as_object_mut) {
            let (id, options) = if up {
                (("id", "modelID"), ("options", "settings"))
            } else {
                (("modelID", "id"), ("settings", "options"))
            };
            rename(model, id.0, id.1);
            rename(model, options.0, options.1);
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allowlists_omit_integrations_and_private_machine_data() {
        for (id, forbidden) in [
            ("opencode", "mcp"),
            ("claude-code", "hooks"),
            ("codex", "mcp_servers"),
            ("gemini-cli", "extensions"),
            ("qwen-code", "channels"),
            ("crush", "recent_models"),
            ("factory", "enabledPlugins"),
        ] {
            assert!(
                !sp_harness::settings(id)
                    .iter()
                    .any(|spec| spec.keys.contains(&forbidden)),
                "{id}"
            );
        }
        assert!(sp_harness::settings("cursor").is_empty());
        assert!(sp_harness::settings("windsurf").is_empty());
    }

    #[test]
    fn setting_item_is_deterministic_and_validated() -> Result<()> {
        let mut setting = Setting {
            harness: "claude-code".into(),
            format: None,
            files: BTreeMap::new(),
        };
        setting.set("settings.json:env.API_KEY", json!("secret"));
        setting.set("settings.json:model", json!("sonnet"));
        let item = setting.item()?;
        let text = String::from_utf8_lossy(&item.files["setting.json"].bytes);
        assert!(text.starts_with("{\n  \"harness\""));
        assert_eq!(
            item.files["setting.json"].bytes,
            setting.item()?.files["setting.json"].bytes
        );
        assert_eq!(
            Setting::from_item(&item)?.entries()["settings.json:env.API_KEY"],
            json!("secret")
        );
        setting.set("settings.json:hooks", json!({"evil":true}));
        assert!(Setting::from_item(&setting.item()?).is_err());
        Ok(())
    }

    #[test]
    fn editors_preserve_unrelated_content() {
        let json = "{\n // keep this\n \"mcp\": {},\n \"model\": \"old\"\n}\n";
        let updated =
            write_entry(json, "settings.json", "model", &json!("new")).unwrap_or_default();
        assert!(updated.contains("// keep this"));
        assert!(updated.contains("\"mcp\": {}"));
        assert!(updated.contains("\"model\": \"new\""));

        let toml = "# keep this\nmodel = \"old\"\n[mcp_servers.foo]\ncommand = \"echo\"\n";
        let updated = write_entry(toml, "config.toml", "model", &json!("new")).unwrap_or_default();
        assert!(updated.contains("# keep this"), "{updated:?}");
        assert!(updated.contains("[mcp_servers.foo]"));
        assert!(updated.contains("model = \"new\""));

        let env = "# keep\nexport GEMINI_API_KEY='old'\n\nOTHER=keep\n";
        assert_eq!(
            write_env(env, "GEMINI_API_KEY", "new"),
            "# keep\nexport GEMINI_API_KEY='new'\n\nOTHER=keep\n"
        );
        assert_eq!(env_entries(env)["GEMINI_API_KEY"], json!("old"));
    }

    #[test]
    fn merge_keeps_local_changes_and_excludes_deletions() {
        let mut pouch = Entries::from([
            ("settings.json:model".into(), json!("remote")),
            ("settings.json:env.KEY".into(), json!("secret")),
        ]);
        let mut device = HarnessState {
            baseline: Entries::from([("settings.json:model".into(), json!("remote"))]),
            ..Default::default()
        };
        let readable = BTreeSet::from(["settings.json".into()]);
        let local = Entries::from([("settings.json:model".into(), json!("local"))]);
        let writes = merge(&mut pouch, &local, &readable, &mut device);
        assert!(writes.contains_key("settings.json:env.KEY"));
        assert_eq!(pouch["settings.json:model"], json!("local"));
        device
            .baseline
            .insert("settings.json:env.KEY".into(), json!("secret"));
        assert!(merge(&mut pouch, &local, &readable, &mut device).is_empty());
        assert!(device.excluded.contains("settings.json:env.KEY"));
        let mut readded = local;
        readded.insert("settings.json:env.KEY".into(), json!("again"));
        merge(&mut pouch, &readded, &readable, &mut device);
        assert!(!device.excluded.contains("settings.json:env.KEY"));
        assert_eq!(pouch["settings.json:env.KEY"], json!("again"));
        pouch.insert("settings.json:model".into(), json!("another computer"));
        assert_eq!(
            merge(&mut pouch, &readded, &readable, &mut device)["settings.json:model"],
            json!("another computer")
        );
    }

    #[test]
    fn opencode_detects_versions_and_translates_providers() {
        assert_eq!(
            detect_opencode("{\"$schema\": \"https://opencode.ai/config/v2/config.json\"}"),
            2
        );
        assert_eq!(detect_opencode("{\"provider\": {}}"), 1);
        assert_eq!(detect_opencode("{}"), 2);
        assert_eq!(version_format("opencode 1.19.4"), 1);
        assert_eq!(version_format("v2.0.1"), 2);
        let original = Entries::from([(
            "opencode.json:providers.mine".into(),
            json!({"name":"Mine","package":"aisdk:@ai-sdk/openai-compatible","settings":{"baseURL":"https://example.com","apiKey":"secret"},"headers":{"x":"y"},"models":{"m1":{"name":"M1","modelID":"m-1","settings":{"reasoning":true}}}}),
        )]);
        let old = translate(&original, Some(2), Some(1));
        let mine = &old["opencode.json:provider.mine"];
        assert_eq!(mine["npm"], json!("@ai-sdk/openai-compatible"));
        assert_eq!(mine["options"]["headers"]["x"], json!("y"));
        assert_eq!(mine["models"]["m1"]["id"], json!("m-1"));
        assert_eq!(mine["models"]["m1"]["options"]["reasoning"], json!(true));
        assert_eq!(translate(&old, Some(1), Some(2)), original);
    }

    #[test]
    fn opencode_terminal_preferences_move_between_tui_and_cli_json() {
        let v1 = Entries::from([
            ("tui.json:theme".into(), json!("tokyonight")),
            ("tui.json:leader_timeout".into(), json!(900)),
            ("tui.json:scroll_speed".into(), json!(3)),
            (
                "tui.json:scroll_acceleration".into(),
                json!({"enabled": true}),
            ),
            ("tui.json:mouse".into(), json!(false)),
            ("tui.json:keybinds".into(), json!({"session_new": "ctrl+n"})),
            ("opencode.json:small_model".into(), json!("mine/m0")),
            ("opencode.json:username".into(), json!("me")),
        ]);
        let v2 = translate(&v1, Some(1), Some(2));
        assert_eq!(
            v2,
            Entries::from([
                ("cli.json:theme".into(), json!({"name": "tokyonight"})),
                ("cli.json:leader".into(), json!({"timeout": 900})),
                (
                    "cli.json:scroll".into(),
                    json!({"speed": 3, "acceleration": true})
                ),
                ("cli.json:mouse".into(), json!(false)),
                ("opencode.json:username".into(), json!("me")),
            ])
        );
        let back = translate(&v2, Some(2), Some(1));
        for key in back.keys() {
            assert_eq!(back[key], v1[key], "{key}");
        }
        assert!(!back.contains_key("tui.json:keybinds"));
    }

    #[test]
    fn an_edit_on_the_other_opencode_version_keeps_the_items_format() -> Result<()> {
        let mut setting = Setting {
            harness: "opencode".into(),
            format: Some(1),
            files: BTreeMap::new(),
        };
        setting.set("opencode.json:model", json!("mine/m1"));
        setting.set("opencode.json:small_model", json!("mine/m0"));
        setting.set("tui.json:theme", json!("tokyonight"));
        put(
            &mut setting,
            &Entries::from([
                ("opencode.json:model".into(), json!("mine/m2")),
                ("cli.json:theme".into(), json!({"name": "gruvbox"})),
                ("cli.json:session".into(), json!({"sidebar": "auto"})),
            ]),
            Some(2),
        );
        let entries = Setting::from_item(&setting.item()?)?.entries();
        assert_eq!(setting.format, Some(1));
        assert_eq!(entries["opencode.json:model"], json!("mine/m2"));
        assert_eq!(entries["opencode.json:small_model"], json!("mine/m0"));
        assert_eq!(entries["tui.json:theme"], json!("gruvbox"));
        assert!(!setting.files.contains_key("cli.json"));
        Ok(())
    }

    #[test]
    fn first_meeting_takes_the_pouch_over_a_fresh_installs_defaults() {
        let readable = BTreeSet::from(["settings.json".into()]);
        let mut pouch = Entries::from([("settings.json:model".into(), json!("from pouch"))]);
        let local = Entries::from([
            ("settings.json:model".into(), json!("fresh default")),
            ("settings.json:theme".into(), json!("only here")),
        ]);
        let mut device = HarnessState::default();
        let writes = merge(&mut pouch, &local, &readable, &mut device);
        assert_eq!(writes["settings.json:model"], json!("from pouch"));
        assert_eq!(pouch["settings.json:model"], json!("from pouch"));
        assert_eq!(pouch["settings.json:theme"], json!("only here"));
        assert_eq!(device.baseline["settings.json:model"], json!("from pouch"));
    }

    #[test]
    fn the_background_sync_watches_every_settings_file() {
        let target = |id: &'static str, root: &str| Target {
            id,
            name: id,
            root: PathBuf::from(root),
            instructions: None,
            skills: None,
            agents: None,
            commands: None,
            mcp: None,
            hooks: None,
            plugins: None,
            read_only: None,
        };
        let opencode = files(&target("opencode", "/oc"));
        for name in ["opencode.json", "opencode.jsonc", "tui.json", "cli.json"] {
            assert!(
                opencode.contains(&PathBuf::from("/oc").join(name)),
                "{name}"
            );
        }
        assert!(files(&target("gemini-cli", "/g")).contains(&PathBuf::from("/g/.env")));
        assert_eq!(
            files(&target("copilot", "/c")),
            [PathBuf::from("/c/settings.json")]
        );
        assert!(files(&target("cursor", "/x")).is_empty());
    }

    #[test]
    fn reinstall_clears_device_agreement_without_changing_pouch() -> Result<()> {
        let temp = tempfile::tempdir().map_err(io(Path::new("temp")))?;
        let root = temp.path().join(".claude");
        fs::create_dir(&root).map_err(io(&root))?;
        let mut setting = Setting {
            harness: "claude-code".into(),
            format: None,
            files: BTreeMap::new(),
        };
        setting.set("settings.json:model", json!("remote"));
        let dir = PouchDir::new(temp.path().join("pouch"));
        dir.write_item("setting", "claude-code", &setting.item()?.files)?;
        let mut state = DeviceState::default();
        state
            .harnesses
            .entry("claude-code".into())
            .or_default()
            .excluded
            .insert("settings.json:model".into());
        state.harnesses.retain(|_, _| root.exists());
        fs::remove_dir(&root).map_err(io(&root))?;
        state.harnesses.retain(|_, _| root.exists());
        assert!(state.harnesses.is_empty());
        fs::create_dir(&root).map_err(io(&root))?;
        let item = dir.read()?.0;
        let target = Target {
            id: "claude-code",
            name: "Claude",
            root: root.clone(),
            instructions: None,
            skills: None,
            agents: None,
            commands: None,
            mcp: None,
            hooks: None,
            plugins: None,
            read_only: None,
        };
        project(
            &dir,
            &item,
            &[target],
            &mut state,
            &temp.path().join("backups"),
        )?;
        assert!(
            fs::read_to_string(root.join("settings.json"))
                .map_err(io(&root))?
                .contains("remote")
        );
        Ok(())
    }

    #[test]
    fn recreated_file_resets_baseline_but_partial_deletion_excludes() -> Result<()> {
        let temp = tempfile::tempdir().map_err(io(Path::new("temp")))?;
        let root = temp.path().join(".claude");
        fs::create_dir(&root).map_err(io(&root))?;
        let file = root.join("settings.json");
        fs::write(&file, "{\"model\":\"new machine default\"}").map_err(io(&file))?;
        let mut setting = Setting {
            harness: "claude-code".into(),
            format: None,
            files: BTreeMap::new(),
        };
        setting.set("settings.json:model", json!("remote"));
        setting.set("settings.json:effortLevel", json!("high"));
        let dir = PouchDir::new(temp.path().join("pouch"));
        dir.write_item("setting", "claude-code", &setting.item()?.files)?;
        let mut state = DeviceState::default();
        state.harnesses.insert(
            "claude-code".into(),
            HarnessState {
                baseline: setting.entries(),
                ..Default::default()
            },
        );
        let target = Target {
            id: "claude-code",
            name: "Claude",
            root: root.clone(),
            instructions: None,
            skills: None,
            agents: None,
            commands: None,
            mcp: None,
            hooks: None,
            plugins: None,
            read_only: None,
        };
        capture(&dir, std::slice::from_ref(&target), &mut state)?;
        project(
            &dir,
            &dir.read()?.0,
            std::slice::from_ref(&target),
            &mut state,
            &temp.path().join("backups"),
        )?;
        assert!(
            state.harnesses["claude-code"]
                .excluded
                .contains("settings.json:effortLevel")
        );

        fs::write(&file, "{}").map_err(io(&file))?;
        capture(&dir, &[target], &mut state)?;
        assert!(state.harnesses["claude-code"].baseline.is_empty());
        assert!(state.harnesses["claude-code"].excluded.is_empty());
        Ok(())
    }

    #[test]
    fn a_folder_made_again_is_a_reinstall_even_with_a_default_value() -> Result<()> {
        let temp = tempfile::tempdir().map_err(io(Path::new("temp")))?;
        let root = temp.path().join(".gemini");
        fs::create_dir(&root).map_err(io(&root))?;
        let Some(first) = created(&root) else {
            return Ok(()); // This file system doesn't record creation times.
        };
        let mut setting = Setting {
            harness: "gemini-cli".into(),
            format: None,
            files: BTreeMap::new(),
        };
        setting.set("settings.json:ui", json!({"theme": "Dracula"}));
        setting.set("settings.json:model", json!({"name": "gem-mine"}));
        setting.set("settings.json:general", json!({"vimMode": true}));
        let dir = PouchDir::new(temp.path().join("pouch"));
        dir.write_item("setting", "gemini-cli", &setting.item()?.files)?;
        let mut state = DeviceState::default();
        state.harnesses.insert(
            "gemini-cli".into(),
            HarnessState {
                baseline: setting.entries(),
                folder_created: Some(first),
                ..Default::default()
            },
        );
        fs::remove_dir_all(&root).map_err(io(&root))?;
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::create_dir(&root).map_err(io(&root))?;
        let file = root.join("settings.json");
        fs::write(&file, "{\"ui\": {\"theme\": \"Default Light\"}}").map_err(io(&file))?;
        let target = Target {
            id: "gemini-cli",
            name: "Gemini CLI",
            root: root.clone(),
            instructions: None,
            skills: None,
            agents: None,
            commands: None,
            mcp: None,
            hooks: None,
            plugins: None,
            read_only: None,
        };
        capture(&dir, std::slice::from_ref(&target), &mut state)?;
        project(
            &dir,
            &dir.read()?.0,
            &[target],
            &mut state,
            &temp.path().join("backups"),
        )?;
        let text = fs::read_to_string(&file).map_err(io(&file))?;
        assert!(text.contains("Dracula") && text.contains("gem-mine") && text.contains("vimMode"));
        assert!(state.harnesses["gemini-cli"].excluded.is_empty());
        assert_ne!(state.harnesses["gemini-cli"].folder_created, Some(first));
        Ok(())
    }

    #[test]
    fn skipped_assistants_remain_in_the_config_after_reinstall() {
        let choice = sp_core::SyncChoice {
            skip_assistants: ["claude-code".into()].into(),
            skip_kinds: BTreeSet::new(),
        };
        assert!(!choice.assistant_on("claude-code"));
        assert!(choice.kind_on("setting"));
    }

    #[test]
    fn fixture_for_installed_cli_config_parsers() -> Result<()> {
        let temp = tempfile::tempdir().map_err(io(Path::new("temp")))?;
        let home = std::env::var_os("SP_SETTINGS_PROBE_HOME")
            .map_or_else(|| temp.path().to_path_buf(), PathBuf::from);
        for (id, folder, entries) in [
            (
                "opencode",
                ".config/opencode",
                Entries::from([("opencode.json:model".into(), json!("openai/gpt-5"))]),
            ),
            (
                "codex",
                ".codex",
                Entries::from([("config.toml:model".into(), json!("gpt-5"))]),
            ),
            (
                "gemini-cli",
                ".gemini",
                Entries::from([("settings.json:ui".into(), json!({"theme": "Default"}))]),
            ),
        ] {
            let root = home.join(folder);
            fs::create_dir_all(&root).map_err(io(&root))?;
            apply(&root, id, &entries, &home.join("backups"))?;
            assert!(root.exists());
        }
        Ok(())
    }

    #[test]
    fn explicit_exclusion_does_not_immediately_uncancel_itself() -> Result<()> {
        let temp = tempfile::tempdir().map_err(io(Path::new("temp")))?;
        let file = temp.path().join("settings.json");
        fs::write(&file, "{\"model\":\"local\"}").map_err(io(&file))?;
        let mut state = HarnessState::default();
        exclude(
            temp.path(),
            "claude-code",
            "settings.json:model",
            &mut state,
        );
        let local = read_local(temp.path(), "claude-code", None).0;
        let mut pouch = Entries::from([("settings.json:model".into(), json!("remote"))]);
        assert!(
            merge(
                &mut pouch,
                &local,
                &BTreeSet::from(["settings.json".into()]),
                &mut state
            )
            .is_empty()
        );
        assert_eq!(pouch["settings.json:model"], json!("remote"));
        assert!(state.excluded.contains("settings.json:model"));
        fs::write(&file, "{\"model\":\"readded\"}").map_err(io(&file))?;
        let local = read_local(temp.path(), "claude-code", None).0;
        merge(
            &mut pouch,
            &local,
            &BTreeSet::from(["settings.json".into()]),
            &mut state,
        );
        assert!(!state.excluded.contains("settings.json:model"));
        assert_eq!(pouch["settings.json:model"], json!("readded"));
        Ok(())
    }

    #[test]
    fn applying_after_another_config_editor_keeps_the_original_backup() -> Result<()> {
        let temp = tempfile::tempdir().map_err(io(Path::new("temp")))?;
        let root = temp.path().join("claude");
        fs::create_dir(&root).map_err(io(&root))?;
        let file = root.join("settings.json");
        fs::write(&file, "{\"model\":\"original\"}").map_err(io(&file))?;
        let backup = temp.path().join("backups");
        let original = back_up(&file, &backup)?;
        fs::write(&file, "{\"model\":\"intermediate\"}").map_err(io(&file))?;
        let saved = apply(
            &root,
            "claude-code",
            &Entries::from([("settings.json:model".into(), json!("new"))]),
            &backup,
        )?;
        assert_eq!(
            fs::read_to_string(original).map_err(io(&file))?,
            "{\"model\":\"original\"}"
        );
        assert_eq!(saved.len(), 1);
        assert!(
            fs::read_to_string(&file)
                .map_err(io(&file))?
                .contains("new")
        );
        Ok(())
    }

    #[test]
    fn nested_allowlist_leaves_security_and_data_directory_alone() -> Result<()> {
        let gemini = specs("gemini-cli", None);
        let spec = gemini
            .iter()
            .find(|spec| spec.file == "settings.json")
            .ok_or_else(|| invalid("missing Gemini settings spec"))?;
        let entries = read_entries("{\"security\":{\"auth\":{\"selectedType\":\"api-key\",\"other\":true},\"folderTrust\":false}}", "gemini-cli", spec).ok_or_else(|| invalid("Gemini parse"))?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries["security.auth.selectedType"], json!("api-key"));
        let mut setting = Setting {
            harness: "gemini-cli".into(),
            format: None,
            files: BTreeMap::new(),
        };
        setting.set("settings.json:security.auth.selectedType", json!("api-key"));
        assert!(Setting::from_item(&setting.item()?).is_ok());
        let text = write_entry(
            "{\"security\":{\"auth\":{\"other\":true},\"folderTrust\":false}}",
            "settings.json",
            "security.auth.selectedType",
            &json!("api-key"),
        )
        .map_err(invalid)?;
        assert!(text.contains("folderTrust"));
        assert!(text.contains("other"));

        let crush = specs("crush", None);
        let entries = read_entries(
            "{\"options\":{\"theme\":\"dark\",\"data_directory\":\"/personal\"}}",
            "crush",
            &crush[0],
        )
        .ok_or_else(|| invalid("Crush parse"))?;
        assert_eq!(entries["options"], json!({"theme":"dark"}));
        let mut setting = Setting {
            harness: "crush".into(),
            format: None,
            files: BTreeMap::new(),
        };
        setting.set("crush.json:options", json!({"data_directory":"/bad"}));
        assert!(Setting::from_item(&setting.item()?).is_err());
        Ok(())
    }
}
