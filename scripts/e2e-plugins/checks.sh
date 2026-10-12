# SPDX-License-Identifier: AGPL-3.0-only
# What each assistant loaded of the two plugins in scripts/e2e-plugins.sh
# (sourced on a computer after harnesses.sh; $M is the mock provider).
# `look_<id>` runs the assistant once, then prints one `<part> <result>`
# line for each of PARTS:
#
#   ok      the assistant itself shows it (its listing, or the part ran and
#           found its plugin's folder); a file on disk only where the
#           assistant has no way to list that part
#   listed  in the assistant's config, but it runs only once the user says
#           so: Codex asks to trust new hooks, and Cursor runs nothing
#           without an account
#   -       the assistant has no such part
#   missing
#
# The kit plugin's hooks and MCP server write to /tmp/marks; a skill counts
# only when both kit's and ponytail's arrived.

PARTS=(skill agent command mcp hook)

cell() {
	local part="$1"
	shift
	if eval "$*" >/dev/null 2>&1; then echo "$part ok"; else echo "$part missing"; fi
}
written() {
	local part="$1"
	shift
	if eval "$*" >/dev/null 2>&1; then echo "$part listed"; else echo "$part missing"; fi
}
none() { echo "$1 -"; }
fresh() {
	rm -rf /tmp/marks
	mkdir -p /tmp/marks
}
ran() { grep -q "^$1 found" /tmp/marks/*.log; }
hooks_ran() { ran SessionStart && ran UserPromptSubmit; }
skills_in() {
	grep -q 'kit skill loaded' "$1/kit-skill/SKILL.md" && grep -q '^name: ponytail$' "$1/ponytail/SKILL.md"
}
agent_in() { grep -q 'kit agent loaded' "$1"; }
command_in() { grep -q 'kit command loaded' "$1"; }
# A TOML command must parse, with the body as its prompt.
toml_command_in() {
	python3 -c 'import sys, tomllib; sys.exit("kit command loaded" not in tomllib.load(open(sys.argv[1], "rb"))["prompt"])' "$1"
}
lists() { grep -q -- "$1" /tmp/listing && grep -q -- "$2" /tmp/listing; }
kit_hook_in() { grep -q "plugins/kit'; sh .*/hooks/mark.sh" "$1"; }

# Claude Code reports what it loaded when a session starts.
init_has() {
	python3 - "$1" "$2" <<'EOF'
import json, sys
key, name = sys.argv[1:]
for line in open('/tmp/init.json'):
    message = json.loads(line)
    if message.get('subtype') == 'init':
        names = [entry if isinstance(entry, str) else entry['name'] for entry in message[key]]
        sys.exit(0 if name in names else 1)
sys.exit(1)
EOF
}
look_claude-code() {
	fresh
	(cd /tmp && timeout 90 claude -p hi --output-format stream-json --verbose 2>/dev/null) |
		grep '"subtype":"init"' >/tmp/init.json || true
	cell skill 'init_has skills kit:kit-skill && init_has skills ponytail:ponytail'
	cell agent 'init_has agents kit:kit-agent'
	cell command 'init_has slash_commands kit:kit-command'
	cell mcp 'init_has mcp_servers plugin:kit:kit-mcp && ran tools'
	cell hook hooks_ran
}

look_codex() {
	fresh
	timeout 90 codex exec --skip-git-repo-check hi >/dev/null 2>&1 || true
	timeout 60 codex mcp list >/tmp/listing 2>&1 || true
	cell skill 'skills_in ~/.agents/skills'
	none agent
	cell command 'command_in ~/.codex/prompts/kit-command.md'
	cell mcp 'lists kit-mcp kit-mcp && ran tools'
	written hook 'kit_hook_in ~/.codex/hooks.json'
}

look_opencode() {
	fresh
	run_opencode
	cell skill 'skills_in ~/.config/opencode/skills'
	cell agent 'agent_in ~/.config/opencode/agents/kit-agent.md'
	cell command 'command_in ~/.config/opencode/commands/kit-command.md'
	cell mcp 'ran tools'
	none hook
}

look_gemini-cli() {
	fresh
	run_gemini-cli
	timeout 60 gemini skills list >/tmp/listing 2>&1 || true
	cell skill 'lists "^kit-skill " "^ponytail "'
	cell agent 'agent_in ~/.gemini/agents/kit-agent.md'
	cell command 'toml_command_in ~/.gemini/commands/kit-command.toml'
	cell mcp 'ran tools'
	cell hook hooks_ran
}

look_qwen-code() {
	fresh
	run_qwen-code
	timeout 60 qwen mcp list >/tmp/listing 2>&1 || true
	cell skill 'skills_in ~/.agents/skills'
	cell agent 'agent_in ~/.qwen/agents/kit-agent.md'
	cell command 'toml_command_in ~/.qwen/commands/kit-command.toml'
	# Its listing connects without asking for tools, and a prompt that the
	# mock provider refuses ends before any server starts.
	cell mcp 'grep "kit-mcp" /tmp/listing | grep -q Connected && ran init'
	cell hook hooks_ran
}

look_copilot() {
	fresh
	(cd /tmp && COPILOT_PROVIDER_BASE_URL="$M/v1" COPILOT_MODEL=m1 \
		timeout 120 copilot -p hi --allow-all-tools >/dev/null 2>&1) || true
	timeout 60 copilot skill list >/tmp/listing 2>&1 || true
	cell skill 'lists "kit-skill - " "ponytail - "'
	cell agent 'agent_in ~/.copilot/agents/kit-agent.md'
	none command
	cell mcp 'ran tools'
	cell hook hooks_ran
}

look_crush() {
	fresh
	run_crush || true
	cell skill 'skills_in ~/.config/crush/skills'
	none agent
	none command
	cell mcp 'ran tools'
	none hook
}

look_factory() {
	fresh
	(cd /tmp && timeout 90 droid exec hi >/dev/null 2>&1) || true
	timeout 60 droid mcp list >/tmp/listing 2>&1 || true
	cell skill 'skills_in ~/.factory/skills'
	cell agent 'agent_in ~/.factory/droids/kit-agent.md'
	cell command 'command_in ~/.factory/commands/kit-command.md'
	cell mcp 'lists kit-mcp kit-mcp && ran tools'
	# `droid exec` runs only SessionStart hooks.
	cell hook 'ran SessionStart && grep -q "mark.sh.* UserPromptSubmit" ~/.factory/settings.json'
}

# Cursor and Antigravity need an account for a prompt: their MCP listing
# starts the servers, and Antigravity starts them before it asks to sign in.
look_cursor() {
	fresh
	# Run in the home folder, Cursor asks before it starts a new server.
	(cd /tmp && timeout 120 cursor-agent mcp list >/tmp/listing 2>&1) || true
	cell skill 'skills_in ~/.cursor/skills'
	none agent
	cell command 'command_in ~/.cursor/commands/kit-command.md'
	cell mcp 'lists "kit-mcp: ready" "kit-mcp: ready" && ran tools'
	written hook 'kit_hook_in ~/.cursor/hooks.json'
}

look_antigravity() {
	fresh
	timeout 60 agy mcp list >/tmp/listing 2>&1 || true
	(cd /tmp && timeout 30 agy -p hi --print-timeout 20s >/dev/null 2>&1) || true
	cell skill 'skills_in ~/.gemini/config/skills'
	cell agent 'agent_in ~/.gemini/config/agents/kit-agent.md'
	none command
	cell mcp 'lists kit-mcp kit-mcp && ran tools'
	none hook
}
