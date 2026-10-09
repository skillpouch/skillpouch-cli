//! MCP servers: the pouch keeps each one as a neutral `server.json`
//! (`{name, description, command, args, env}` or `{name, description, type,
//! url, headers}`); each assistant gets it in its own config format.
//!
//! Only the servers SkillPouch wrote are ever changed or removed in an
//! assistant's config; everything else in the file stays as it was.

use crate::error::io;
use crate::{Result, SyncError};
use serde_json::{Map, Value, json};
use sp_harness::McpFormat;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// A `server.json` as a value, `name` and `description` included.
pub type Server = Value;

fn object(value: &Value, key: &str) -> Map<String, Value> {
    value
        .get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn strings(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter(|v| v.is_string()).cloned().collect())
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// `server.json` from an assistant's config entry; `None` if it defines
/// nothing that runs.
pub fn from_native(format: McpFormat, name: &str, entry: &Value) -> Option<Server> {
    let description = text(entry, "description").unwrap_or_default();
    let remote = |url: String, kind: &str, headers: Map<String, Value>| json!({ "name": name, "description": description, "type": kind, "url": url, "headers": headers });
    let local = |command: String, args: Vec<Value>, env: Map<String, Value>| json!({ "name": name, "description": description, "command": command, "args": args, "env": env });
    match format {
        McpFormat::OpenCode => {
            if let Some(url) = text(entry, "url") {
                return Some(remote(url, "http", object(entry, "headers")));
            }
            let mut parts = strings(entry, "command").into_iter();
            let command = parts.next()?.as_str()?.to_string();
            Some(local(
                command,
                parts.collect(),
                object(entry, "environment"),
            ))
        }
        McpFormat::Gemini => {
            if let Some(url) = text(entry, "httpUrl") {
                return Some(remote(url, "http", object(entry, "headers")));
            }
            if let Some(url) = text(entry, "url") {
                return Some(remote(url, "sse", object(entry, "headers")));
            }
            let command = text(entry, "command")?;
            Some(local(command, strings(entry, "args"), object(entry, "env")))
        }
        McpFormat::Codex => {
            if let Some(url) = text(entry, "url") {
                return Some(remote(url, "http", object(entry, "http_headers")));
            }
            let command = text(entry, "command")?;
            Some(local(command, strings(entry, "args"), object(entry, "env")))
        }
        McpFormat::Windsurf => {
            if let Some(url) = text(entry, "serverUrl").or_else(|| text(entry, "url")) {
                return Some(remote(url, "http", object(entry, "headers")));
            }
            let command = text(entry, "command")?;
            Some(local(command, strings(entry, "args"), object(entry, "env")))
        }
        McpFormat::Claude | McpFormat::Copilot => {
            if let Some(url) = text(entry, "url") {
                let kind = if text(entry, "type").as_deref() == Some("sse") {
                    "sse"
                } else {
                    "http"
                };
                return Some(remote(url, kind, object(entry, "headers")));
            }
            let command = text(entry, "command")?;
            Some(local(command, strings(entry, "args"), object(entry, "env")))
        }
    }
}

/// The assistant's config entry for a `server.json`; `None` if it's broken.
pub fn to_native(format: McpFormat, server: &Server) -> Option<Value> {
    let headers = object(server, "headers");
    let env = object(server, "env");
    let args = strings(server, "args");
    if let Some(url) = text(server, "url") {
        let sse = text(server, "type").as_deref() == Some("sse");
        return Some(match format {
            McpFormat::Claude => {
                json!({ "type": if sse { "sse" } else { "http" }, "url": url, "headers": headers })
            }
            McpFormat::OpenCode => {
                json!({ "type": "remote", "url": url, "headers": headers, "enabled": true })
            }
            McpFormat::Codex => json!({ "url": url, "http_headers": headers }),
            McpFormat::Gemini if sse => json!({ "url": url, "headers": headers }),
            McpFormat::Gemini => json!({ "httpUrl": url, "headers": headers }),
            McpFormat::Copilot => json!({
                "type": if sse { "sse" } else { "http" },
                "url": url,
                "headers": headers,
                "tools": ["*"],
            }),
            McpFormat::Windsurf => json!({ "serverUrl": url, "headers": headers }),
        });
    }
    let command = text(server, "command").filter(|c| !c.is_empty())?;
    Some(match format {
        McpFormat::OpenCode => {
            let mut line = vec![Value::String(command)];
            line.extend(args);
            json!({ "type": "local", "command": line, "environment": env, "enabled": true })
        }
        McpFormat::Claude => {
            json!({ "type": "stdio", "command": command, "args": args, "env": env })
        }
        McpFormat::Copilot => {
            json!({ "type": "local", "command": command, "args": args, "env": env, "tools": ["*"] })
        }
        McpFormat::Codex | McpFormat::Gemini | McpFormat::Windsurf => {
            json!({ "command": command, "args": args, "env": env })
        }
    })
}

fn key_of(format: McpFormat) -> &'static str {
    format.key()
}

/// The servers defined in an assistant's config file, as `server.json`s.
pub fn read(path: &Path, format: McpFormat) -> BTreeMap<String, Server> {
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let entries: Map<String, Value> = match format {
        McpFormat::Codex => text
            .parse::<toml::Table>()
            .ok()
            .and_then(|table| table.get(key_of(format)).cloned())
            .and_then(|value| serde_json::to_value(value).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        _ => serde_json::from_str::<Value>(&sp_harness::strip_jsonc(&text))
            .ok()
            .map(|value| object(&value, key_of(format)))
            .unwrap_or_default(),
    };
    entries
        .iter()
        .filter_map(|(name, entry)| Some((name.clone(), from_native(format, name, entry)?)))
        .collect()
}

/// What writing a config file did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Written {
    /// The servers SkillPouch now owns in that file.
    pub managed: BTreeSet<String>,
    /// Why the file was left alone, if it was.
    pub skipped: Option<String>,
}

/// Puts `servers` into the config file and removes the ones SkillPouch put
/// there before (`managed`) that are gone. Writes only when something changes.
pub fn write(
    path: &Path,
    format: McpFormat,
    servers: &BTreeMap<String, Server>,
    managed: &BTreeSet<String>,
) -> Result<Written> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io(path)(error)),
    };
    let native: BTreeMap<String, Value> = servers
        .iter()
        .filter_map(|(name, server)| Some((name.clone(), to_native(format, server)?)))
        .collect();
    let updated = match format {
        McpFormat::Codex => write_toml(&text, &native, managed),
        _ => write_json(&text, key_of(format), &native, managed),
    };
    let updated = match updated {
        Ok(updated) => updated,
        Err(reason) => {
            return Ok(Written {
                managed: managed.clone(),
                skipped: Some(format!("{}: {reason}", path.display())),
            });
        }
    };
    if updated != text {
        crate::item::write_file(path, &crate::item::File::new(updated.into_bytes()))?;
        restrict_like_before(path);
    }
    Ok(Written {
        managed: native.keys().cloned().collect(),
        skipped: None,
    })
}

/// Configs can hold API keys: keep them private to this user.
fn restrict_like_before(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn write_json(
    text: &str,
    key: &str,
    native: &BTreeMap<String, Value>,
    managed: &BTreeSet<String>,
) -> std::result::Result<String, String> {
    if text.trim().is_empty() && native.is_empty() {
        return Ok(text.to_string());
    }
    crate::jsonc::set_entries(text, key, native, managed)
}

fn write_toml(
    text: &str,
    native: &BTreeMap<String, Value>,
    managed: &BTreeSet<String>,
) -> std::result::Result<String, String> {
    use toml_edit::{DocumentMut, Item, Table};
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|_| "it isn't valid TOML".to_string())?;
    if !doc.contains_key("mcp_servers") {
        if native.is_empty() {
            return Ok(text.to_string());
        }
        let mut table = Table::new();
        table.set_implicit(true);
        doc.insert("mcp_servers", Item::Table(table));
    }
    let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
        return Err("`mcp_servers` isn't a table".into());
    };
    for name in managed.iter().filter(|name| !native.contains_key(*name)) {
        servers.remove(name);
    }
    for (name, entry) in native {
        let wanted = toml_table(entry);
        let same = servers
            .get(name)
            .and_then(Item::as_table)
            .is_some_and(|current| current.to_string() == wanted.to_string());
        if !same {
            servers.insert(name, Item::Table(wanted));
        }
    }
    Ok(doc.to_string())
}

fn toml_table(entry: &Value) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    if let Some(fields) = entry.as_object() {
        for (key, value) in fields {
            if let Some(value) = toml_value(value) {
                table.insert(key, toml_edit::Item::Value(value));
            }
        }
    }
    table
}

fn toml_value(value: &Value) -> Option<toml_edit::Value> {
    Some(match value {
        Value::String(text) => text.as_str().into(),
        Value::Bool(flag) => (*flag).into(),
        Value::Number(number) => number.as_i64()?.into(),
        Value::Array(items) => {
            toml_edit::Value::Array(items.iter().filter_map(toml_value).collect())
        }
        Value::Object(fields) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, value) in fields {
                if let Some(value) = toml_value(value) {
                    inline.insert(key, value);
                }
            }
            toml_edit::Value::InlineTable(inline)
        }
        Value::Null => return None,
    })
}

/// A `server.json` file's contents.
pub fn parse_server(bytes: &[u8]) -> Result<Server> {
    serde_json::from_slice(bytes)
        .map_err(|error| SyncError::Invalid(format!("server.json isn't valid JSON: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio() -> Server {
        json!({ "name": "gh", "description": "", "command": "npx", "args": ["-y", "gh-mcp"], "env": { "TOKEN": "x" } })
    }

    #[test]
    fn every_format_round_trips() {
        let remote = json!({ "name": "docs", "description": "", "type": "sse", "url": "https://x", "headers": {} });
        for format in [
            McpFormat::Claude,
            McpFormat::OpenCode,
            McpFormat::Codex,
            McpFormat::Gemini,
        ] {
            for server in [stdio(), remote.clone()] {
                let native = to_native(format, &server).unwrap_or_default();
                let back =
                    from_native(format, server["name"].as_str().unwrap_or_default(), &native);
                let mut expected = server.clone();
                // Only Claude and Gemini tell SSE from streamable HTTP.
                if matches!(format, McpFormat::OpenCode | McpFormat::Codex)
                    && server.get("url").is_some()
                {
                    expected["type"] = json!("http");
                }
                assert_eq!(back, Some(expected), "{format:?}");
            }
        }
        assert_eq!(to_native(McpFormat::Claude, &json!({ "name": "x" })), None);
    }

    #[test]
    fn json_keeps_other_settings_and_removes_only_its_own() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("claude.json");
        fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"mine":{"command":"a"},"old":{"command":"b"}}}"#,
        )
        .map_err(io(&path))?;
        let mut servers = BTreeMap::new();
        servers.insert("gh".to_string(), stdio());
        let managed = BTreeSet::from(["old".to_string()]);
        let written = write(&path, McpFormat::Claude, &servers, &managed)?;
        assert_eq!(written.managed, BTreeSet::from(["gh".to_string()]));
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).map_err(io(&path))?)
            .unwrap_or_default();
        assert_eq!(value["theme"], "dark");
        assert!(value["mcpServers"]["mine"].is_object());
        assert!(value["mcpServers"].get("old").is_none());
        assert_eq!(value["mcpServers"]["gh"]["command"], "npx");
        assert_eq!(read(&path, McpFormat::Claude).len(), 2);
        Ok(())
    }

    #[test]
    fn comments_survive_in_toml_and_jsonc() -> Result<()> {
        let dir = tempfile::tempdir().map_err(io(Path::new(".")))?;
        let path = dir.path().join("config.toml");
        fs::write(&path, "# my settings\nmodel = \"gpt\"\n").map_err(io(&path))?;
        let mut servers = BTreeMap::new();
        servers.insert("gh".to_string(), stdio());
        write(&path, McpFormat::Codex, &servers, &BTreeSet::new())?;
        let text = fs::read_to_string(&path).map_err(io(&path))?;
        assert!(
            text.starts_with("# my settings\nmodel = \"gpt\"\n"),
            "{text}"
        );
        assert_eq!(read(&path, McpFormat::Codex).get("gh"), Some(&stdio()));
        // Writing the same again changes nothing.
        write(
            &path,
            McpFormat::Codex,
            &servers,
            &BTreeSet::from(["gh".into()]),
        )?;
        assert_eq!(fs::read_to_string(&path).map_err(io(&path))?, text);

        let jsonc = dir.path().join("opencode.jsonc");
        fs::write(&jsonc, "{\n  // mine\n  \"theme\": \"x\",\n}\n").map_err(io(&jsonc))?;
        let written = write(&jsonc, McpFormat::OpenCode, &servers, &BTreeSet::new())?;
        assert_eq!(written.skipped, None);
        let text = fs::read_to_string(&jsonc).map_err(io(&jsonc))?;
        assert!(text.contains("// mine"), "{text}");
        assert!(
            read(&jsonc, McpFormat::OpenCode).contains_key("gh"),
            "{text}"
        );
        Ok(())
    }
}
