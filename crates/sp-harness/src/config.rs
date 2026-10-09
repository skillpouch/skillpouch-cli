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
    value
        .get(key)
        .and_then(serde_json::Value::as_object)
        .map(json_servers)
        .unwrap_or_default()
}

/// The servers in a JSON map. Some configs nest them one level deeper,
/// under `servers`.
fn json_servers(map: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let mut names = Vec::new();
    for (name, server) in map {
        let Some(fields) = server.as_object() else {
            continue;
        };
        let defines = ["type", "command", "url", "httpUrl"]
            .iter()
            .any(|field| fields.contains_key(*field));
        if name == "servers" && !defines {
            names.extend(json_servers(fields));
        } else {
            names.push(name.clone());
        }
    }
    names
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
        // A server that happens to be called `servers` is still one.
        assert_eq!(
            json_keys(r#"{"mcp":{"servers":{"type":"local"}}}"#, "mcp"),
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
