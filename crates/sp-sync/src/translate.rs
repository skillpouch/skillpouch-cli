//! Agents and commands in each assistant's own dialect. The pouch keeps
//! them the way Claude Code writes them (`AGENT.md`, `COMMAND.md`); every
//! other assistant gets a copy translated to what it reads: its front matter
//! keys, its tool names, its argument placeholder. Edits made to a copy come
//! back translated the other way, keeping whatever the assistant can't say
//! (a Claude model name in OpenCode, say) as it was.

use crate::command::{self, yaml_scalar};
use sp_harness::{AgentFormat, CommandFormat};

/// A front matter value, as far as these files use YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    List(Vec<String>),
    Map(Vec<(String, String)>),
}

/// Front matter keys in order, and the body after it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Doc {
    front: Vec<(String, Value)>,
    body: String,
}

impl Doc {
    fn parse(text: &str) -> Self {
        let Some(rest) = text
            .strip_prefix("---\n")
            .or_else(|| text.strip_prefix("---\r\n"))
        else {
            return Self {
                front: Vec::new(),
                body: text.to_string(),
            };
        };
        let mut offset = 0;
        let mut end = None;
        for line in rest.split_inclusive('\n') {
            if line.trim_end() == "---" {
                end = Some((offset, offset + line.len()));
                break;
            }
            offset += line.len();
        }
        let Some((front_end, body_start)) = end else {
            return Self {
                front: Vec::new(),
                body: text.to_string(),
            };
        };
        let body = rest[body_start..]
            .trim_start_matches(['\r', '\n'])
            .to_string();
        let lines: Vec<&str> = rest[..front_end].lines().collect();
        let mut front = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            index += 1;
            let Some((key, raw)) = line.split_once(':') else {
                continue;
            };
            if key.starts_with([' ', '\t']) || key.trim().is_empty() {
                continue;
            }
            let (key, raw) = (key.trim().to_string(), raw.trim());
            let indented: Vec<&str> = lines[index..]
                .iter()
                .take_while(|next| next.starts_with([' ', '\t']) || next.trim().is_empty())
                .copied()
                .collect();
            let value = if raw.is_empty() || raw == "|" || raw == ">" {
                index += indented.len();
                let items: Vec<&str> = indented
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();
                if raw.is_empty() && !items.is_empty() && items.iter().all(|l| l.starts_with("- "))
                {
                    Value::List(items.iter().map(|l| unquote(&l[2..])).collect())
                } else if raw.is_empty()
                    && !items.is_empty()
                    && items.iter().all(|l| l.contains(':'))
                {
                    Value::Map(
                        items
                            .iter()
                            .filter_map(|l| l.split_once(':'))
                            .map(|(k, v)| (unquote(k), unquote(v)))
                            .collect(),
                    )
                } else {
                    let separator = if raw == ">" { " " } else { "\n" };
                    Value::Text(items.join(separator))
                }
            } else if let Some(inner) = raw
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .filter(|_| key == "tools")
            {
                Value::List(
                    inner
                        .split(',')
                        .map(unquote)
                        .filter(|item| !item.is_empty())
                        .collect(),
                )
            } else {
                Value::Text(unquote(raw))
            };
            front.push((key, value));
        }
        Self { front, body }
    }

    fn render(&self) -> String {
        let mut out = String::new();
        if !self.front.is_empty() {
            out.push_str("---\n");
            for (key, value) in &self.front {
                match value {
                    Value::Text(text) if text.contains('\n') => {
                        out.push_str(&format!("{key}: |\n"));
                        for line in text.lines() {
                            out.push_str(&format!("  {line}\n"));
                        }
                    }
                    Value::Text(text) => out.push_str(&format!("{key}: {}\n", yaml_scalar(text))),
                    Value::List(items) => {
                        out.push_str(&format!("{key}:\n"));
                        for item in items {
                            out.push_str(&format!("  - {}\n", yaml_scalar(item)));
                        }
                    }
                    Value::Map(pairs) => {
                        out.push_str(&format!("{key}:\n"));
                        for (k, v) in pairs {
                            out.push_str(&format!("  {k}: {v}\n"));
                        }
                    }
                }
            }
            out.push_str("---\n\n");
        }
        out.push_str(&self.body);
        out
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.front.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn text(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::Text(text) if !text.is_empty() => Some(text.clone()),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: Option<Value>) {
        let at = self.front.iter().position(|(k, _)| k == key);
        match (at, value) {
            (Some(at), Some(value)) => self.front[at].1 = value,
            (None, Some(value)) => self.front.push((key.to_string(), value)),
            (Some(at), None) => {
                self.front.remove(at);
            }
            (None, None) => {}
        }
    }

    /// Only these keys, in this order.
    fn keep(&self, keys: &[&str]) -> Vec<(String, Value)> {
        keys.iter()
            .filter_map(|key| self.get(key).map(|v| (key.to_string(), v.clone())))
            .collect()
    }
}

fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('"')
        && let Ok(text) = serde_json::from_str::<String>(raw)
    {
        return text;
    }
    raw.trim_matches(['"', '\'']).to_string()
}

/// Claude Code's tools and what each assistant calls them:
/// (Claude, OpenCode, Gemini CLI, Copilot CLI, Factory).
const TOOLS: [ToolRow; 11] = [
    (
        "Read",
        Some("read"),
        Some("read_file"),
        Some("read"),
        Some("Read"),
    ),
    (
        "Write",
        Some("write"),
        Some("write_file"),
        Some("edit"),
        Some("Create"),
    ),
    (
        "Edit",
        Some("edit"),
        Some("replace"),
        Some("edit"),
        Some("Edit"),
    ),
    (
        "MultiEdit",
        Some("edit"),
        Some("replace"),
        Some("edit"),
        Some("Edit"),
    ),
    (
        "Bash",
        Some("bash"),
        Some("run_shell_command"),
        Some("shell"),
        Some("Execute"),
    ),
    (
        "Grep",
        Some("grep"),
        Some("search_file_content"),
        Some("search"),
        Some("Grep"),
    ),
    (
        "Glob",
        Some("glob"),
        Some("glob"),
        Some("search"),
        Some("Glob"),
    ),
    (
        "LS",
        Some("list"),
        Some("list_directory"),
        Some("search"),
        Some("LS"),
    ),
    (
        "WebFetch",
        Some("webfetch"),
        Some("web_fetch"),
        Some("web"),
        Some("FetchUrl"),
    ),
    (
        "WebSearch",
        None,
        Some("google_web_search"),
        Some("web"),
        Some("WebSearch"),
    ),
    (
        "TodoWrite",
        Some("todowrite"),
        Some("write_todos"),
        Some("todo"),
        Some("TodoWrite"),
    ),
];

/// Every tool OpenCode has; an agent turns off the ones it may not use.
const OPENCODE_TOOLS: [&str; 11] = [
    "bash",
    "edit",
    "write",
    "read",
    "grep",
    "glob",
    "list",
    "patch",
    "todowrite",
    "todoread",
    "webfetch",
];

type ToolRow = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

fn column(format: AgentFormat, row: &ToolRow) -> Option<&'static str> {
    match format {
        AgentFormat::Claude => Some(row.0),
        AgentFormat::OpenCode => row.1,
        AgentFormat::Gemini => row.2,
        AgentFormat::Copilot => row.3,
        AgentFormat::Factory => row.4,
    }
}

/// The Claude tools an agent may use; `None`: all of them.
fn claude_tools(doc: &Doc) -> Option<Vec<String>> {
    let names: Vec<String> = match doc.get("tools")? {
        Value::Text(text) => text
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        Value::List(items) => items.clone(),
        Value::Map(_) => return None,
    };
    (!names.is_empty()).then_some(names)
}

fn tools_to(format: AgentFormat, tools: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tool in tools {
        let native = TOOLS
            .iter()
            .find(|row| row.0.eq_ignore_ascii_case(tool))
            .and_then(|row| column(format, row))
            .map(str::to_string)
            // A tool only Claude knows (an MCP tool, `Task`) keeps its name.
            .unwrap_or_else(|| tool.clone());
        if !out.contains(&native) {
            out.push(native);
        }
    }
    out
}

fn tools_from(format: AgentFormat, native: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in native {
        let claude: Vec<String> = TOOLS
            .iter()
            .filter(|row| column(format, row) == Some(name.as_str()))
            .map(|row| row.0.to_string())
            .collect();
        let claude = if claude.is_empty() {
            vec![name.clone()]
        } else {
            claude
        };
        for tool in claude {
            if !out.contains(&tool) {
                out.push(tool);
            }
        }
    }
    out
}

/// The tools a native agent may use, in Claude's names; `None`: all.
fn native_tools(format: AgentFormat, doc: &Doc) -> Option<Vec<String>> {
    match format {
        AgentFormat::Claude => claude_tools(doc),
        AgentFormat::OpenCode => {
            let Some(Value::Map(pairs)) = doc.get("tools") else {
                return None;
            };
            let off: Vec<&str> = pairs
                .iter()
                .filter(|(_, on)| on.trim() == "false")
                .map(|(tool, _)| tool.as_str())
                .collect();
            if off.is_empty() {
                return None;
            }
            // `patch` and `todoread` go with `edit` and `todowrite`.
            let on: Vec<String> = OPENCODE_TOOLS
                .iter()
                .filter(|tool| !off.contains(tool) && !matches!(**tool, "patch" | "todoread"))
                .map(|tool| tool.to_string())
                .collect();
            Some(tools_from(format, &on))
        }
        _ => {
            let names = match doc.get("tools")? {
                Value::List(items) => items.clone(),
                Value::Text(text) => text.split(',').map(|t| t.trim().to_string()).collect(),
                Value::Map(_) => return None,
            };
            Some(tools_from(format, &names))
        }
    }
}

/// `AGENT.md` as assistant `format` reads it.
pub fn agent_to(format: AgentFormat, neutral: &str, name: &str) -> String {
    let doc = Doc::parse(neutral);
    if format == AgentFormat::Claude {
        return neutral.to_string();
    }
    let description = doc.text("description").unwrap_or_else(|| name.to_string());
    let tools = claude_tools(&doc);
    let mut front: Vec<(String, Value)> = Vec::new();
    match format {
        AgentFormat::OpenCode => {
            front.push(("description".into(), Value::Text(description)));
            front.push(("mode".into(), Value::Text("subagent".into())));
            if let Some(model) = doc.text("model").filter(|m| m.contains('/')) {
                front.push(("model".into(), Value::Text(model)));
            }
            front.extend(doc.keep(&["temperature"]));
            if let Some(tools) = tools {
                let on = tools_to(format, &tools);
                let mut off: Vec<(String, String)> = OPENCODE_TOOLS
                    .iter()
                    .filter(|tool| {
                        let covered = |name: &str| on.iter().any(|t| t == name);
                        match **tool {
                            "patch" => !covered("edit"),
                            "todoread" => !covered("todowrite"),
                            other => !covered(other),
                        }
                    })
                    .map(|tool| (tool.to_string(), "false".to_string()))
                    .collect();
                off.sort();
                if !off.is_empty() {
                    front.push(("tools".into(), Value::Map(off)));
                }
            }
        }
        AgentFormat::Gemini | AgentFormat::Copilot | AgentFormat::Factory => {
            front.push((
                "name".into(),
                Value::Text(doc.text("name").unwrap_or_else(|| name.to_string())),
            ));
            front.push(("description".into(), Value::Text(description)));
            if format == AgentFormat::Factory {
                front.push(("model".into(), Value::Text("inherit".into())));
            }
            if format == AgentFormat::Gemini {
                if let Some(model) = doc.text("model").filter(|m| m.starts_with("gemini")) {
                    front.push(("model".into(), Value::Text(model)));
                }
                front.extend(doc.keep(&["temperature", "max_turns"]));
            }
            if let Some(tools) = tools {
                front.push(("tools".into(), Value::List(tools_to(format, &tools))));
            }
        }
        AgentFormat::Claude => {}
    }
    Doc {
        front,
        body: doc.body,
    }
    .render()
}

/// An agent as edited in assistant `format`, back as `AGENT.md`. What the
/// assistant can't express is kept from `current` (empty when the agent is
/// new to the pouch; `name` names it then).
pub fn agent_from(format: AgentFormat, native: &str, current: &str, name: &str) -> String {
    if format == AgentFormat::Claude || agent_to(format, current, name) == native {
        return if format == AgentFormat::Claude {
            native
        } else {
            current
        }
        .to_string();
    }
    let edited = Doc::parse(native);
    let was = Doc::parse(&agent_to(format, current, name));
    let mut doc = Doc::parse(current);
    if doc.text("name").is_none() {
        let name = edited.text("name").unwrap_or_else(|| name.to_string());
        doc.front.insert(0, ("name".into(), Value::Text(name)));
    }
    if edited.text("description") != was.text("description") {
        doc.set("description", edited.text("description").map(Value::Text));
    }
    let tools = native_tools(format, &edited);
    if tools != native_tools(format, &was) {
        doc.set("tools", tools.map(|tools| Value::Text(tools.join(", "))));
    }
    doc.body = edited.body;
    doc.render()
}

/// The same agent or command, written differently (quotes, blank lines,
/// a tool list instead of a comma string).
pub fn same_document(a: &str, b: &str) -> bool {
    let normal = |text: &str| {
        let mut doc = Doc::parse(text);
        doc.body = doc.body.trim().to_string();
        for (key, value) in &mut doc.front {
            if key == "tools"
                && let Value::Text(text) = value
            {
                *value = Value::List(text.split(',').map(|t| t.trim().to_string()).collect());
            }
        }
        doc
    };
    normal(a) == normal(b)
}

/// `COMMAND.md` as assistant `format` reads it.
pub fn command_to(format: CommandFormat, neutral: &str) -> String {
    let doc = Doc::parse(neutral);
    let front = match format {
        CommandFormat::Markdown => return neutral.to_string(),
        CommandFormat::Toml => return command::to_toml(neutral),
        CommandFormat::Plain => Vec::new(),
        CommandFormat::Workflow => doc.keep(&["description"]),
        CommandFormat::Hinted => doc.keep(&["description", "argument-hint"]),
        CommandFormat::OpenCode => {
            let mut front = doc.keep(&["description", "agent"]);
            if let Some(model) = doc.text("model").filter(|m| m.contains('/')) {
                front.push(("model".into(), Value::Text(model)));
            }
            front
        }
    };
    Doc {
        front,
        body: doc.body,
    }
    .render()
}

/// A command as edited in assistant `format`, back as `COMMAND.md`.
pub fn command_from(format: CommandFormat, native: &str, current: &str) -> Option<String> {
    match format {
        CommandFormat::Markdown => return Some(native.to_string()),
        CommandFormat::Toml => return command::from_toml(native, current),
        _ => {}
    }
    if command_to(format, current) == native {
        return Some(current.to_string());
    }
    let edited = Doc::parse(native);
    let was = Doc::parse(&command_to(format, current));
    let mut doc = Doc::parse(current);
    for key in ["description", "argument-hint", "agent"] {
        if edited.get(key) != was.get(key) {
            doc.set(key, edited.get(key).cloned());
        }
    }
    doc.body = edited.body;
    Some(doc.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "---\nname: reviewer\ndescription: Reviews diffs before merging\ntools: Read, Grep, Glob, Bash\nmodel: sonnet\ncolor: blue\n---\n\nYou review code.\n";

    #[test]
    fn the_same_document_written_differently_is_the_same() {
        assert!(same_document(
            "---\nname: a\ntools: Read, Grep\n---\nBody\n",
            "---\nname: \"a\"\ntools:\n  - Read\n  - Grep\n---\n\nBody"
        ));
        assert!(!same_document(
            "---\nname: a\n---\nBody",
            "---\nname: b\n---\nBody"
        ));
    }

    #[test]
    fn agents_speak_each_assistants_dialect() {
        let opencode = agent_to(AgentFormat::OpenCode, AGENT, "reviewer");
        assert_eq!(
            opencode,
            "---\ndescription: Reviews diffs before merging\nmode: subagent\ntools:\n  edit: false\n  list: false\n  patch: false\n  todoread: false\n  todowrite: false\n  webfetch: false\n  write: false\n---\n\nYou review code.\n"
        );
        let gemini = agent_to(AgentFormat::Gemini, AGENT, "reviewer");
        assert!(
            gemini.contains(
                "tools:\n  - read_file\n  - search_file_content\n  - glob\n  - run_shell_command\n"
            ),
            "{gemini}"
        );
        assert!(!gemini.contains("model"), "{gemini}");
        let factory = agent_to(AgentFormat::Factory, AGENT, "reviewer");
        assert!(
            factory.contains("model: inherit\ntools:\n  - Read\n  - Grep\n  - Glob\n  - Execute\n"),
            "{factory}"
        );
        let copilot = agent_to(AgentFormat::Copilot, AGENT, "reviewer");
        assert!(
            copilot.contains("tools:\n  - read\n  - search\n  - shell\n"),
            "{copilot}"
        );
        assert_eq!(agent_to(AgentFormat::Claude, AGENT, "reviewer"), AGENT);
    }

    #[test]
    fn unchanged_copies_give_the_pouch_file_back_untouched() {
        for format in [
            AgentFormat::OpenCode,
            AgentFormat::Gemini,
            AgentFormat::Copilot,
            AgentFormat::Factory,
        ] {
            let native = agent_to(format, AGENT, "reviewer");
            assert_eq!(
                agent_from(format, &native, AGENT, "reviewer"),
                AGENT,
                "{format:?}"
            );
        }
    }

    #[test]
    fn edits_in_a_copy_come_back_keeping_the_rest() {
        let native = agent_to(AgentFormat::Gemini, AGENT, "reviewer")
            .replace("  - run_shell_command\n", "")
            .replace("You review code.", "You review code carefully.");
        let back = agent_from(AgentFormat::Gemini, &native, AGENT, "reviewer");
        assert!(back.contains("tools: Read, Grep, Glob\n"), "{back}");
        // Gemini can't hold these: they stay as they were.
        assert!(back.contains("model: sonnet\ncolor: blue\n"), "{back}");
        assert!(back.ends_with("You review code carefully.\n"), "{back}");

        let opencode =
            agent_to(AgentFormat::OpenCode, AGENT, "reviewer").replace("  bash: false\n", "");
        let opencode = opencode.replace(
            "description: Reviews diffs before merging",
            "description: Reviews every diff",
        );
        let back = agent_from(AgentFormat::OpenCode, &opencode, AGENT, "reviewer");
        assert!(back.contains("description: Reviews every diff\n"), "{back}");
        assert!(back.contains("name: reviewer\n"), "{back}");
    }

    #[test]
    fn adopted_agents_get_a_name_and_claude_tools() {
        let native = "---\ndescription: Plans work\nmode: subagent\ntools:\n  write: false\n  edit: false\n  bash: false\n  patch: false\n---\n\nPlan it.\n";
        let neutral = agent_from(AgentFormat::OpenCode, native, "", "planner");
        assert!(
            neutral.starts_with("---\nname: planner\ndescription: Plans work\ntools: "),
            "{neutral}"
        );
        assert!(
            neutral.contains("Read") && !neutral.contains("Bash"),
            "{neutral}"
        );
    }

    const COMMAND: &str = "---\ndescription: Commit staged work\nargument-hint: [message]\nallowed-tools: Bash(git commit:*)\nmodel: haiku\n---\n\nCommit with message $ARGUMENTS\n";

    #[test]
    fn commands_speak_each_assistants_dialect() {
        assert_eq!(
            command_to(CommandFormat::OpenCode, COMMAND),
            "---\ndescription: Commit staged work\n---\n\nCommit with message $ARGUMENTS\n"
        );
        assert_eq!(
            command_to(CommandFormat::Hinted, COMMAND),
            "---\ndescription: Commit staged work\nargument-hint: \"[message]\"\n---\n\nCommit with message $ARGUMENTS\n"
        );
        assert_eq!(
            command_to(CommandFormat::Plain, COMMAND),
            "Commit with message $ARGUMENTS\n"
        );
        assert!(command_to(CommandFormat::Toml, COMMAND).contains("{{args}}"));
        for format in [
            CommandFormat::OpenCode,
            CommandFormat::Hinted,
            CommandFormat::Plain,
            CommandFormat::Workflow,
        ] {
            let native = command_to(format, COMMAND);
            assert_eq!(
                command_from(format, &native, COMMAND).as_deref(),
                Some(COMMAND),
                "{format:?}"
            );
        }
        let edited = command_to(CommandFormat::Plain, COMMAND).replace("Commit", "Amend");
        let back = command_from(CommandFormat::Plain, &edited, COMMAND).unwrap_or_default();
        assert!(
            back.contains("allowed-tools: Bash(git commit:*)\n")
                && back.ends_with("Amend with message $ARGUMENTS\n"),
            "{back}"
        );
    }
}
