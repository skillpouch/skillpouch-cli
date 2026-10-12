//! MCP servers, read from each assistant's config file.

use crate::{Item, Kind};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Format {
    /// JSON with comments and trailing commas (OpenCode).
    Jsonc,
    Toml,
}

/// Config files to read, the first that exists winning, and the key whose
/// entries are the servers.
pub(crate) struct Source {
    pub files: Vec<PathBuf>,
    pub format: Format,
    pub key: &'static str,
}

pub(crate) fn mcp_servers(source: &Source) -> Vec<Item> {
    let Some((path, text)) = source
        .files
        .iter()
        .find_map(|path| Some((path, std::fs::read_to_string(path).ok()?)))
    else {
        return Vec::new();
    };
    let names = match source.format {
        Format::Jsonc => json_keys(&strip_jsonc(&text), source.key),
        Format::Toml => toml_keys(&text, source.key),
    };
    names
        .into_iter()
        .map(|name| Item {
            kind: Kind::McpServer,
            name,
            path: path.clone(),
            files: 0,
            bytes: 0,
        })
        .collect()
}

fn json_keys(text: &str, key: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    mcp_entries(&value, key).keys().cloned().collect()
}

type Map = serde_json::Map<String, serde_json::Value>;

/// The `servers` object under an assistant's MCP key, if the servers are
/// nested one level deeper (OpenCode 2's `mcp.servers`). A server that
/// happens to be called `servers` doesn't count.
pub fn nested_servers(map: &Map) -> Option<&Map> {
    let fields = map.get("servers")?.as_object()?;
    let defines = ["type", "command", "url", "httpUrl", "serverUrl"]
        .iter()
        .any(|field| fields.contains_key(*field));
    (!defines).then_some(fields)
}

/// The server entries under `key` in a parsed config, by name: the ones
/// nested under `servers` and any written beside it.
pub fn mcp_entries(value: &serde_json::Value, key: &str) -> Map {
    let Some(map) = value.get(key).and_then(serde_json::Value::as_object) else {
        return Map::new();
    };
    let nested = nested_servers(map);
    let mut entries: Map = map
        .iter()
        .filter(|(name, server)| {
            server.is_object() && !(nested.is_some() && name.as_str() == "servers")
        })
        .map(|(name, server)| (name.clone(), server.clone()))
        .collect();
    for (name, server) in nested.into_iter().flatten() {
        if server.is_object() {
            entries.insert(name.clone(), server.clone());
        }
    }
    entries
}

fn toml_keys(text: &str, key: &str) -> Vec<String> {
    let Ok(value) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    value
        .get(key)
        .and_then(toml::Value::as_table)
        .map(|servers| {
            servers
                .iter()
                .filter(|(_, server)| server.is_table())
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Plain JSON from JSONC: comments, then trailing commas, removed; strings
/// left alone.
pub fn strip_jsonc(text: &str) -> String {
    drop_trailing_commas(&drop_comments(text))
}

/// Calls `outside` for every character that isn't inside a string literal
/// (with the index), and copies string literals unchanged.
fn rewrite(text: &str, mut outside: impl FnMut(&[char], &mut usize, &mut String)) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '"' {
            outside(&chars, &mut index, &mut out);
            continue;
        }
        out.push('"');
        index += 1;
        while index < chars.len() {
            let char = chars[index];
            out.push(char);
            index += 1;
            if char == '\\' {
                if let Some(&escaped) = chars.get(index) {
                    out.push(escaped);
                    index += 1;
                }
            } else if char == '"' {
                break;
            }
        }
    }
    out
}

fn drop_comments(text: &str) -> String {
    rewrite(text, |chars, index, out| {
        match (chars[*index], chars.get(*index + 1)) {
            ('/', Some('/')) => {
                while *index < chars.len() && chars[*index] != '\n' {
                    *index += 1;
                }
            }
            ('/', Some('*')) => {
                *index += 2;
                while *index < chars.len()
                    && !(chars[*index] == '*' && chars.get(*index + 1) == Some(&'/'))
                {
                    *index += 1;
                }
                *index += 2;
            }
            (char, _) => {
                out.push(char);
                *index += 1;
            }
        }
    })
}

fn drop_trailing_commas(text: &str) -> String {
    rewrite(text, |chars, index, out| {
        let char = chars[*index];
        let closes = chars[*index + 1..]
            .iter()
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| matches!(c, '}' | ']'));
        if !(char == ',' && closes) {
            out.push(char);
        }
        *index += 1;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_servers_nested_under_servers() {
        let flat = json_keys(r#"{"mcp":{"fs":{"type":"local"}}}"#, "mcp");
        let nested = json_keys(
            r#"{"mcp":{"servers":{"a":{"command":["x"]},"b":{}}}}"#,
            "mcp",
        );
        assert_eq!(flat, ["fs"]);
        assert_eq!(nested, ["a", "b"]);
        // Written both ways in one file, both count; values that aren't
        // servers don't.
        let mut both = json_keys(
            r#"{"mcp":{"timeout":5,"old":{"command":["y"]},"servers":{"a":{}}}}"#,
            "mcp",
        );
        both.sort();
        assert_eq!(both, ["a", "old"]);
        // A server that happens to be called `servers` is still one.
        assert_eq!(
            json_keys(r#"{"mcp":{"servers":{"type":"local"}}}"#, "mcp"),
            ["servers"]
        );
        assert_eq!(
            json_keys(
                r#"{"mcpServers":{"servers":{"serverUrl":"https://x"}}}"#,
                "mcpServers"
            ),
            ["servers"]
        );
    }

    #[test]
    fn strips_comments_and_trailing_commas_but_not_strings() {
        let plain = strip_jsonc(
            "{ /* a */ \"u\": \"http://x/*y*/\", // b\n \"l\": [1, 2, // c\n], \"q\": \"a\\\"//\", }",
        );
        let value: serde_json::Value = serde_json::from_str(&plain).unwrap_or_default();
        assert_eq!(value["u"], "http://x/*y*/");
        assert_eq!(value["l"], serde_json::json!([1, 2]));
        assert_eq!(value["q"], "a\"//");
    }
}
