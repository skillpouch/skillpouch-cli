//! Editing one object inside a JSON or JSONC file in place: entries are
//! replaced, added or removed as text, so comments, key order and the
//! file's own formatting stay as they were everywhere else.

use serde_json::Value;
use std::collections::BTreeMap;

/// One `"name": value` inside an object.
#[derive(Debug, Clone)]
struct Member {
    name: String,
    /// Where the key's opening quote is.
    start: usize,
    value_start: usize,
    value_end: usize,
    /// The comma after the value, if any.
    comma: Option<usize>,
}

/// An object's members and where its braces are.
#[derive(Debug)]
struct Object {
    open: usize,
    close: usize,
    members: Vec<Member>,
}

type Parsed<T> = std::result::Result<T, String>;

fn skip_blank(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) == Some(&b'/') && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b.get(i) == Some(&b'/') && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else {
            return i;
        }
    }
}

/// The index after a string starting at `i` (on its quote).
fn string_end(b: &[u8], mut i: usize) -> Parsed<usize> {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err("an unterminated string".into())
}

/// The index after the value starting at `i`.
fn value_end(b: &[u8], i: usize) -> Parsed<usize> {
    match b.get(i) {
        Some(b'"') => string_end(b, i),
        Some(b'{') => object(b, i).map(|object| object.close + 1),
        Some(b'[') => {
            let mut j = skip_blank(b, i + 1);
            while b.get(j) != Some(&b']') {
                j = skip_blank(b, value_end(b, j)?);
                match b.get(j) {
                    Some(b',') => j = skip_blank(b, j + 1),
                    Some(b']') => {}
                    _ => return Err("a broken array".into()),
                }
            }
            Ok(j + 1)
        }
        Some(_) => {
            let mut j = i;
            while j < b.len()
                && !matches!(b[j], b',' | b'}' | b']' | b'/')
                && !b[j].is_ascii_whitespace()
            {
                j += 1;
            }
            if j == i {
                Err("a missing value".into())
            } else {
                Ok(j)
            }
        }
        None => Err("an unexpected end".into()),
    }
}

fn object(b: &[u8], open: usize) -> Parsed<Object> {
    let mut members = Vec::new();
    let mut i = skip_blank(b, open + 1);
    while b.get(i) != Some(&b'}') {
        if b.get(i) != Some(&b'"') {
            return Err("an object key that isn't a string".into());
        }
        let start = i;
        let key_end = string_end(b, i)?;
        let name: String =
            serde_json::from_slice(&b[start..key_end]).map_err(|_| "a broken key".to_string())?;
        i = skip_blank(b, key_end);
        if b.get(i) != Some(&b':') {
            return Err("a key without a value".into());
        }
        let value_start = skip_blank(b, i + 1);
        let end = value_end(b, value_start)?;
        i = skip_blank(b, end);
        let comma = (b.get(i) == Some(&b',')).then_some(i);
        if comma.is_some() {
            i = skip_blank(b, i + 1);
        } else if b.get(i) != Some(&b'}') {
            return Err("a missing comma".into());
        }
        members.push(Member {
            name,
            start,
            value_start,
            value_end: end,
            comma,
        });
    }
    Ok(Object {
        open,
        close: i,
        members,
    })
}

/// The whitespace a line starts with.
fn indent_at(text: &str, at: usize) -> &str {
    let line = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let rest = &text[line..];
    &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
}

/// The file's indent unit: a tab or the smallest run of leading spaces.
fn indent_unit(text: &str) -> String {
    let mut smallest: Option<usize> = None;
    for line in text.lines() {
        if line.starts_with('\t') {
            return "\t".into();
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if spaces > 0 && line.len() > spaces {
            smallest = Some(smallest.map_or(spaces, |s| s.min(spaces)));
        }
    }
    " ".repeat(smallest.unwrap_or(2))
}

/// `value` pretty-printed, its later lines indented by `base`.
fn render(value: &Value, unit: &str, base: &str) -> String {
    use serde::Serialize;
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    if value.serialize(&mut serializer).is_err() {
        return "null".into();
    }
    String::from_utf8_lossy(&out).replace('\n', &format!("\n{base}"))
}

fn value_of(text: &str, member: &Member) -> Option<Value> {
    serde_json::from_str(&sp_harness::strip_jsonc(
        &text[member.value_start..member.value_end],
    ))
    .ok()
}

/// Sets the object at `path` (keys from the top) to hold `entries` for the
/// names SkillPouch owns: `managed` names not in `entries` are removed,
/// others are left as the user has them. Returns the new text.
pub fn set_entries(
    text: &str,
    path: &[&str],
    entries: &BTreeMap<String, Value>,
    managed: &std::collections::BTreeSet<String>,
) -> Parsed<String> {
    let unit = indent_unit(text);
    let mut text = if text.trim().is_empty() {
        "{}\n".to_string()
    } else {
        text.to_string()
    };
    let root_open = skip_blank(text.as_bytes(), 0);
    if text.as_bytes().get(root_open) != Some(&b'{') {
        return Err("it isn't a JSON object".into());
    }
    let Some(servers) = open_path(&mut text, root_open, path, !entries.is_empty(), &unit)? else {
        return Ok(text);
    };

    // Changes inside the object, applied one at a time on fresh offsets.
    for name in managed.iter().filter(|name| !entries.contains_key(*name)) {
        let current = object(text.as_bytes(), servers)?;
        if let Some(index) = current.members.iter().position(|m| &m.name == name) {
            text = remove_member(&text, &current, index);
        }
    }
    for (name, value) in entries {
        let current = object(text.as_bytes(), servers)?;
        let base = member_indent(&text, &current, &unit);
        match current.members.iter().find(|m| &m.name == name) {
            Some(member) if value_of(&text, member).as_ref() == Some(value) => {}
            Some(member) => {
                let rendered = render(value, &unit, &base);
                text.replace_range(member.value_start..member.value_end, &rendered);
            }
            None => {
                let rendered = format!(
                    "{}: {}",
                    serde_json::to_string(name).unwrap_or_default(),
                    render(value, &unit, &base)
                );
                text = insert_member(&text, &current, &rendered, &base, &unit);
            }
        }
    }
    Ok(text)
}

/// Sets the value at `path` (object keys from the top), or removes it with
/// `None`. Missing objects on the way are created; everything else in the
/// file stays as written.
pub fn set_value(text: &str, path: &[&str], value: Option<&Value>) -> Parsed<String> {
    let Some((last, parents)) = path.split_last() else {
        return Err("an empty path".into());
    };
    let unit = indent_unit(text);
    let mut text = if text.trim().is_empty() {
        if value.is_none() {
            return Ok(text.to_string());
        }
        "{}\n".to_string()
    } else {
        text.to_string()
    };
    let root = skip_blank(text.as_bytes(), 0);
    if text.as_bytes().get(root) != Some(&b'{') {
        return Err("it isn't a JSON object".into());
    }
    let Some(at) = open_path(&mut text, root, parents, value.is_some(), &unit)? else {
        return Ok(text);
    };
    let current = object(text.as_bytes(), at)?;
    let base = member_indent(&text, &current, &unit);
    let index = current.members.iter().position(|m| m.name == *last);
    match (index, value) {
        (None, None) => {}
        (Some(index), None) => text = remove_member(&text, &current, index),
        (Some(index), Some(value)) => {
            let member = &current.members[index];
            if value_of(&text, member).as_ref() != Some(value) {
                let rendered = render(value, &unit, &base);
                text.replace_range(member.value_start..member.value_end, &rendered);
            }
        }
        (None, Some(value)) => {
            let rendered = format!(
                "{}: {}",
                serde_json::to_string(last).unwrap_or_default(),
                render(value, &unit, &base)
            );
            text = insert_member(&text, &current, &rendered, &base, &unit);
        }
    }
    Ok(text)
}

/// Where the object at `path` (keys from the object at `at`) opens, adding
/// the missing objects on the way when `create`; `None` if one is missing.
fn open_path(
    text: &mut String,
    mut at: usize,
    path: &[&str],
    create: bool,
    unit: &str,
) -> Parsed<Option<usize>> {
    for key in path {
        let current = object(text.as_bytes(), at)?;
        match current.members.iter().find(|m| m.name == *key) {
            Some(member) if text.as_bytes()[member.value_start] == b'{' => at = member.value_start,
            Some(_) => return Err(format!("`{key}` isn't an object")),
            None if !create => return Ok(None),
            None => {
                let base = member_indent(text, &current, unit);
                let member = format!("{}: {{}}", serde_json::to_string(key).unwrap_or_default());
                *text = insert_member(text, &current, &member, &base, unit);
                let current = object(text.as_bytes(), at)?;
                at = current
                    .members
                    .iter()
                    .find(|m| m.name == *key)
                    .map(|m| m.value_start)
                    .ok_or("couldn't add the key")?;
            }
        }
    }
    Ok(Some(at))
}

/// How the members of `object` are indented (one level in from its line).
fn member_indent(text: &str, object: &Object, unit: &str) -> String {
    match object.members.first() {
        Some(first)
            if text[..first.start].rfind('\n') > Some(object.open)
                || text[object.open..first.start].contains('\n') =>
        {
            indent_at(text, first.start).to_string()
        }
        _ => format!("{}{unit}", indent_at(text, object.open)),
    }
}

fn insert_member(text: &str, object: &Object, member: &str, base: &str, unit: &str) -> String {
    let base = if base.is_empty() {
        member_indent(text, object, unit)
    } else {
        base.to_string()
    };
    let mut out = text.to_string();
    match object.members.last() {
        Some(last) => {
            let (at, prefix) = match last.comma {
                Some(comma) => (comma + 1, ""),
                None => (last.value_end, ","),
            };
            out.insert_str(at, &format!("{prefix}\n{base}{member}"));
        }
        None => {
            let close_indent = indent_at(text, object.open).to_string();
            out.replace_range(
                object.open + 1..object.close,
                &format!("\n{base}{member}\n{close_indent}"),
            );
        }
    }
    out
}

fn remove_member(text: &str, object: &Object, index: usize) -> String {
    let member = &object.members[index];
    let line_start = text[..member.start].rfind('\n').map_or(0, |n| n + 1);
    let start = if text[line_start..member.start].trim().is_empty() {
        line_start
    } else {
        member.start
    };
    let mut end = member.comma.map_or(member.value_end, |c| c + 1);
    let is_last = index + 1 == object.members.len();
    let mut out = text.to_string();
    if is_last && index > 0 {
        // Drop the comma before it, so the object doesn't end in one.
        let previous = &object.members[index - 1];
        if let Some(comma) = previous.comma
            && member.comma.is_none()
        {
            end = end.max(member.value_end);
            out.replace_range(start.max(comma)..end, "");
            out.replace_range(comma..comma + 1, "");
            return trim_blank_line(out, comma);
        }
    }
    if text[end..].starts_with('\n') && start == line_start {
        end += 1;
    }
    out.replace_range(start..end, "");
    out
}

fn trim_blank_line(mut text: String, at: usize) -> String {
    let line_end = text[at..].find('\n').map_or(text.len(), |n| at + n);
    if text[at..line_end].trim().is_empty() && line_end < text.len() {
        text.replace_range(at..line_end, "");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn entries(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn parsed(text: &str) -> Value {
        serde_json::from_str(&sp_harness::strip_jsonc(text)).unwrap_or_default()
    }

    #[test]
    fn adds_replaces_and_removes_keeping_comments() -> Parsed<()> {
        let text = "{\n  // my model\n  \"model\": \"x\", /* keep */\n  \"mcp\": {\n    // mine\n    \"fs\": { \"type\": \"local\" },\n    \"old\": { \"type\": \"local\" },\n  },\n}\n";
        let managed = BTreeSet::from(["old".to_string(), "gh".to_string()]);
        let out = set_entries(
            text,
            &["mcp"],
            &entries(&[("gh", json!({"type": "remote", "url": "u"}))]),
            &managed,
        )?;
        assert!(
            out.contains("// my model") && out.contains("/* keep */") && out.contains("// mine"),
            "{out}"
        );
        let value = parsed(&out);
        assert_eq!(value["mcp"]["gh"]["url"], "u");
        assert!(value["mcp"].get("old").is_none());
        assert_eq!(value["mcp"]["fs"]["type"], "local");
        assert_eq!(value["model"], "x");

        // Same again: nothing changes.
        assert_eq!(
            set_entries(
                &out,
                &["mcp"],
                &entries(&[("gh", json!({"type": "remote", "url": "u"}))]),
                &managed
            )?,
            out
        );
        // Changed: only that value is rewritten.
        let changed = set_entries(
            &out,
            &["mcp"],
            &entries(&[("gh", json!({"url": "v"}))]),
            &managed,
        )?;
        assert_eq!(parsed(&changed)["mcp"]["gh"], json!({"url": "v"}));
        assert!(changed.contains("// mine"));
        Ok(())
    }

    #[test]
    fn creates_the_key_and_the_file() -> Parsed<()> {
        let out = set_entries(
            "",
            &["mcpServers"],
            &entries(&[("a", json!({"command": "x"}))]),
            &BTreeSet::new(),
        )?;
        assert_eq!(parsed(&out)["mcpServers"]["a"]["command"], "x");
        let out = set_entries(
            "{\n\t\"theme\": 1\n}",
            &["mcpServers"],
            &entries(&[("a", json!({"command": "x"}))]),
            &BTreeSet::new(),
        )?;
        assert!(out.contains("\n\t\"mcpServers\""), "{out}");
        assert_eq!(parsed(&out)["theme"], 1);
        Ok(())
    }

    #[test]
    fn sets_entries_in_a_nested_object() -> Parsed<()> {
        let path = ["mcp", "servers"];
        let a = entries(&[("a", json!({"command": ["x"]}))]);
        let out = set_entries(
            "{\n  // mine\n  \"model\": \"x\"\n}\n",
            &path,
            &a,
            &BTreeSet::new(),
        )?;
        assert!(out.contains("// mine"), "{out}");
        assert_eq!(parsed(&out)["mcp"]["servers"]["a"]["command"], json!(["x"]));
        assert_eq!(set_entries(&out, &path, &a, &BTreeSet::new())?, out);
        // Nothing to set, nothing created.
        assert_eq!(
            set_entries("{}", &path, &BTreeMap::new(), &BTreeSet::new())?,
            "{}"
        );
        Ok(())
    }

    #[test]
    fn removing_the_last_entry_leaves_valid_json() -> Parsed<()> {
        let text = "{\"mcpServers\": {\n  \"a\": {},\n  \"b\": {}\n}}";
        let out = set_entries(
            text,
            &["mcpServers"],
            &BTreeMap::new(),
            &BTreeSet::from(["b".to_string()]),
        )?;
        let value: Value = serde_json::from_str(&out).map_err(|e| format!("{e}: {out}"))?;
        assert_eq!(value["mcpServers"], json!({"a": {}}));
        let out = set_entries(
            &out,
            &["mcpServers"],
            &BTreeMap::new(),
            &BTreeSet::from(["a".to_string()]),
        )?;
        let value: Value = serde_json::from_str(&out).map_err(|e| format!("{e}: {out}"))?;
        assert_eq!(value["mcpServers"], json!({}));
        Ok(())
    }

    #[test]
    fn sets_nested_values_and_removes_them() -> Parsed<()> {
        let text = "{\n  // mine\n  \"model\": \"x\"\n}\n";
        let groups = json!([{"hooks": [{"type": "command", "command": "fmt"}]}]);
        let out = set_value(text, &["hooks", "PostToolUse"], Some(&groups))?;
        assert!(out.contains("// mine"), "{out}");
        assert_eq!(parsed(&out)["hooks"]["PostToolUse"], groups);
        assert_eq!(
            set_value(&out, &["hooks", "PostToolUse"], Some(&groups))?,
            out
        );
        let out = set_value(&out, &["hooks", "PostToolUse"], None)?;
        assert_eq!(parsed(&out)["hooks"], json!({}));
        assert_eq!(parsed(&out)["model"], "x");
        // Nothing to remove, nothing created.
        assert_eq!(set_value("", &["a", "b"], None)?, "");
        assert!(set_value("{\"hooks\": 1}", &["hooks", "x"], Some(&json!(1))).is_err());
        Ok(())
    }

    #[test]
    fn refuses_what_it_cannot_parse() {
        assert!(set_entries("[1]", &["mcp"], &BTreeMap::new(), &BTreeSet::new()).is_err());
        assert!(
            set_entries(
                "{\"mcp\": 3}",
                &["mcp"],
                &entries(&[("a", json!({}))]),
                &BTreeSet::new()
            )
            .is_err()
        );
        assert!(set_entries("{\"a\" 1}", &["mcp"], &BTreeMap::new(), &BTreeSet::new()).is_err());
    }
}
