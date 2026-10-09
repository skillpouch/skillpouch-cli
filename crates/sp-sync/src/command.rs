//! Commands in the two shapes assistants read them:
//!
//! - `COMMAND.md` (the pouch, Claude Code, OpenCode, …): optional front
//!   matter with `description`, then the prompt; arguments as `$ARGUMENTS`.
//! - `<name>.toml` (Gemini CLI, Qwen Code): `description` and `prompt`;
//!   arguments as `{{args}}`.

const MD_ARGS: &str = "$ARGUMENTS";
const TOML_ARGS: &str = "{{args}}";

/// Front matter lines (without the `---` fences) and the body after them.
fn split(markdown: &str) -> (Option<Vec<&str>>, &str) {
    let Some(rest) = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))
    else {
        return (None, markdown);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let lines = rest[..offset].lines().collect();
            return (Some(lines), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, markdown)
}

fn yaml_value(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('"')
        && let Ok(text) = serde_json::from_str::<String>(raw)
    {
        return text;
    }
    raw.trim_matches(['"', '\'']).to_string()
}

/// `text` as a YAML value: plain when that reads back the same, else quoted.
pub(crate) fn yaml_scalar(text: &str) -> String {
    let plain = !text.is_empty()
        && text.trim() == text
        && !text.contains(": ")
        && !text.contains(" #")
        && !text.ends_with(':')
        && !text.contains(['\n', '"'])
        && !text.starts_with(|c: char| "!&*-?|>'%@`{}[],#\"".contains(c))
        && !matches!(
            text.to_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "~"
        )
        && text.parse::<f64>().is_err();
    if plain {
        text.to_string()
    } else {
        serde_json::to_string(text).unwrap_or_default()
    }
}

fn description_of(lines: &[&str]) -> Option<String> {
    lines
        .iter()
        .find_map(|line| line.strip_prefix("description:"))
        .map(yaml_value)
        .filter(|text| !text.is_empty())
}

/// `COMMAND.md` → the TOML file Gemini CLI reads.
pub fn to_toml(markdown: &str) -> String {
    let (front, body) = split(markdown);
    let mut table = toml::Table::new();
    if let Some(description) = front.as_deref().and_then(description_of) {
        table.insert("description".into(), description.into());
    }
    let prompt = body
        .trim_start_matches(['\r', '\n'])
        .replace(MD_ARGS, TOML_ARGS);
    table.insert("prompt".into(), prompt.into());
    toml::to_string(&table).unwrap_or_default()
}

/// A TOML command (as edited in Gemini CLI) back into `COMMAND.md`. The
/// rest of `current`'s front matter is kept; `None` if the TOML is broken.
pub fn from_toml(text: &str, current: &str) -> Option<String> {
    let table: toml::Table = text.parse().ok()?;
    let prompt = table.get("prompt")?.as_str()?.replace(TOML_ARGS, MD_ARGS);
    let description = table
        .get("description")
        .and_then(toml::Value::as_str)
        .filter(|text| !text.is_empty());
    let (front, _) = split(current);
    let front = front.unwrap_or_default();
    // The same description keeps its line as written.
    let kept = front
        .iter()
        .find(|line| line.starts_with("description:"))
        .filter(|line| description.is_some_and(|d| yaml_value(&line["description:".len()..]) == d))
        .map(|line| line.to_string());
    let mut lines: Vec<String> = front
        .into_iter()
        .filter(|line| !line.starts_with("description:"))
        .map(str::to_string)
        .collect();
    if let Some(description) = description {
        lines.insert(
            0,
            kept.unwrap_or_else(|| format!("description: {}", yaml_scalar(description))),
        );
    }
    let mut out = String::new();
    if !lines.is_empty() {
        out.push_str("---\n");
        for line in &lines {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("---\n\n");
    }
    out.push_str(&prompt);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trips_to_the_same_markdown() {
        for md in [
            "---\ndescription: Ship it\n---\n\nShip $ARGUMENTS\n",
            "---\ndescription: \"Fix: the bug\"\n---\n\nFix\n",
        ] {
            assert_eq!(from_toml(&to_toml(md), "").as_deref(), Some(md), "{md}");
            assert_eq!(from_toml(&to_toml(md), md).as_deref(), Some(md), "{md}");
        }
        assert_eq!(yaml_scalar("yes"), "\"yes\"");
        assert_eq!(yaml_scalar("Review the diff"), "Review the diff");
    }

    #[test]
    fn markdown_becomes_toml_with_gemini_arguments() {
        let md = "---\ndescription: \"Fix: the bug\"\nmodel: x\n---\n\nFix $ARGUMENTS now.\n";
        let toml = to_toml(md);
        let table: toml::Table = toml.parse().unwrap_or_default();
        assert_eq!(table["description"].as_str(), Some("Fix: the bug"));
        assert_eq!(table["prompt"].as_str(), Some("Fix {{args}} now.\n"));
        // No front matter: just a prompt.
        let plain: toml::Table = to_toml("Say hi").parse().unwrap_or_default();
        assert_eq!(plain.get("description"), None);
        assert_eq!(plain["prompt"].as_str(), Some("Say hi"));
    }

    #[test]
    fn edits_in_toml_come_back_keeping_other_front_matter() {
        let md = "---\ndescription: old\nmodel: x\n---\n\nOld $ARGUMENTS\n";
        let edited = "description = \"new one\"\nprompt = \"New {{args}}\\n\"\n";
        let back = from_toml(edited, md).unwrap_or_default();
        assert_eq!(
            back,
            "---\ndescription: new one\nmodel: x\n---\n\nNew $ARGUMENTS\n"
        );
        // And round-trips to the same TOML.
        assert_eq!(
            to_toml(&back),
            to_toml(&from_toml(&to_toml(&back), &back).unwrap_or_default())
        );
        assert_eq!(from_toml("prompt = 3", md), None);
        assert_eq!(from_toml("prompt = \"x\"", "").as_deref(), Some("x"));
    }
}
