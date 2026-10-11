# SPDX-License-Identifier: AGPL-3.0-only
# Per-assistant fixtures for scripts/e2e-settings.sh (sourced; $M is the mock
# provider's base URL). For each assistant id:
#
#   ROOT[id]      its folder, relative to $HOME
#   PKG[id]       npm package (uninstalled and installed again offline)
#   seed_<id>     computer A's config: a custom provider, base URL, key and
#                 model, a UI preference, and things that are never settings
#   default_<id>  a fresh install's config: a comment and a key that isn't
#                 synced, both of which must survive
#   CHECK[id]     entries that must arrive: `file|path|json` lines
#   ABSENT[id]    what must never arrive: `file|path` lines
#   LOCAL[id]     the fresh install's own values that must survive
#   COMMENT[id]   `file|text` comment that must survive, if the format has them
#   EDIT[id]      an edit made on B: `file|path|json` lines
#   GONE[id]      an entry deleted on B: `file|path|entry`
#   SECRET[id]    a value share links must blank
#   run_<id>      the real CLI using the synced settings: fails when its own
#                 config check does; its call to the mock provider fails by
#                 design and is judged from the mock's log
# opencode has v1 (A, C) and v2 (B) variants: CHECK[opencode@2] and so on.

IDS=(opencode claude-code codex gemini-cli qwen-code copilot crush factory)
declare -A ROOT PKG CHECK ABSENT LOCAL COMMENT EDIT GONE SECRET

ROOT[opencode]=.config/opencode
ROOT[claude-code]=.claude
ROOT[codex]=.codex
ROOT[gemini-cli]=.gemini
ROOT[qwen-code]=.qwen
ROOT[copilot]=.copilot
ROOT[crush]=.config/crush
ROOT[factory]=.factory

PKG[opencode]=opencode-ai@1.19.0
PKG[claude-code]=@anthropic-ai/claude-code@2.1.296
PKG[codex]=@openai/codex@0.162.1
PKG[gemini-cli]=@google/gemini-cli@0.63.0
PKG[qwen-code]=@qwen-code/qwen-code@0.25.0
PKG[copilot]=@github/copilot@1.0.95
PKG[crush]=@charmland/crush@0.98.1
PKG[factory]=droid@0.238.0

# --- OpenCode ----------------------------------------------------------------
seed_opencode() {
	cat >"$HOME/.config/opencode/opencode.json" <<EOF
{
  // A's OpenCode
  "\$schema": "https://opencode.ai/config.json",
  "provider": {
    "mine": {
      "name": "Mine",
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "$M/v1", "apiKey": "sk-e2e-opencode" },
      "models": { "m1": { "name": "M1" }, "m2": { "name": "M2" } }
    }
  },
  "model": "mine/m1",
  "mcp": { "e2e-tool": { "type": "local", "command": ["echo", "hi"] } },
  "plugin": ["opencode-e2e-plugin"]
}
EOF
	cat >"$HOME/.config/opencode/tui.json" <<EOF
{
  "\$schema": "https://opencode.ai/tui.json",
  "theme": "tokyonight",
  "plugin": ["tui-e2e-plugin"]
}
EOF
}
default_opencode() {
	printf '{\n  // B'"'"'s OpenCode\n  "snapshots": false\n}\n' >"$HOME/.config/opencode/opencode.json"
}
CHECK[opencode@1]="opencode.json|provider.mine.options.baseURL|\"$M/v1\"
opencode.json|provider.mine.options.apiKey|\"sk-e2e-opencode\"
opencode.json|provider.mine.npm|\"@ai-sdk/openai-compatible\"
opencode.json|model|\"mine/m1\"
tui.json|theme|\"tokyonight\""
CHECK[opencode@2]="opencode.json|providers.mine.settings.baseURL|\"$M/v1\"
opencode.json|providers.mine.settings.apiKey|\"sk-e2e-opencode\"
opencode.json|model|\"mine/m1\"
cli.json|theme.name|\"tokyonight\""
ABSENT[opencode@1]="opencode.json|mcp
opencode.json|plugin
tui.json|plugin"
ABSENT[opencode@2]="opencode.json|mcp
opencode.json|plugin
opencode.json|plugins
cli.json|plugins
tui.json|plugin"
LOCAL[opencode]="opencode.json|snapshots|false"
COMMENT[opencode]="opencode.json|// B's OpenCode"
EDIT[opencode@2]="opencode.json|model|\"mine/m2\"
opencode.json|providers.mine.settings.baseURL|\"$M/v2\""
EDIT[opencode@1]="opencode.json|model|\"mine/m2\"
opencode.json|provider.mine.options.baseURL|\"$M/v2\""
GONE[opencode@2]="cli.json|theme|cli.json:theme"
GONE[opencode@1]="tui.json|theme|tui.json:theme"
SECRET[opencode]=sk-e2e-opencode
run_opencode() {
	if opencode --version | grep -q 'v2\.'; then
		timeout 120 opencode run --standalone hi >/dev/null 2>&1 || true
	else
		timeout 120 opencode run hi >/dev/null 2>&1 || true
	fi
}

# --- Claude Code -------------------------------------------------------------
seed_claude-code() {
	cat >"$HOME/.claude/settings.json" <<EOF
{
  "env": { "ANTHROPIC_BASE_URL": "$M", "ANTHROPIC_AUTH_TOKEN": "sk-e2e-claude" },
  "model": "mine-sonnet",
  "theme": "dark-daltonized",
  "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "echo stop" }] }] },
  "enabledPlugins": { "e2e@market": true }
}
EOF
	echo '{"claudeAiOauth":{"accessToken":"sk-ant-oat01-e2e-credential"}}' >"$HOME/.claude/.credentials.json"
}
default_claude-code() {
	printf '{\n  "autoMemoryDirectory": "/root/b-memory"\n}\n' >"$HOME/.claude/settings.json"
}
CHECK[claude-code]="settings.json|env.ANTHROPIC_BASE_URL|\"$M\"
settings.json|env.ANTHROPIC_AUTH_TOKEN|\"sk-e2e-claude\"
settings.json|model|\"mine-sonnet\"
settings.json|theme|\"dark-daltonized\""
ABSENT[claude-code]="settings.json|hooks
settings.json|enabledPlugins
.credentials.json|claudeAiOauth"
LOCAL[claude-code]="settings.json|autoMemoryDirectory|\"/root/b-memory\""
EDIT[claude-code]="settings.json|model|\"mine-opus\""
GONE[claude-code]="settings.json|theme|settings.json:theme"
SECRET[claude-code]=sk-e2e-claude
run_claude-code() { timeout 90 claude -p hi >/dev/null 2>&1 || true; }

# --- Codex -------------------------------------------------------------------
seed_codex() {
	cat >"$HOME/.codex/config.toml" <<EOF
# A's Codex
model = "m1"
model_provider = "mine"
model_reasoning_effort = "high"

[model_providers.mine]
name = "Mine"
base_url = "$M/v1"
experimental_bearer_token = "sk-e2e-codex"
wire_api = "responses"

[tui]
notifications = true

[mcp_servers.e2e]
command = "echo"

[projects."/root/work"]
trust_level = "trusted"
EOF
	echo '{"OPENAI_API_KEY":"sk-e2e-codex-auth-json"}' >"$HOME/.codex/auth.json"
}
default_codex() {
	printf '# B'"'"'s Codex\n[projects."/srv"]\ntrust_level = "trusted"\n' >"$HOME/.codex/config.toml"
}
CHECK[codex]="config.toml|model|\"m1\"
config.toml|model_provider|\"mine\"
config.toml|model_reasoning_effort|\"high\"
config.toml|model_providers.mine.base_url|\"$M/v1\"
config.toml|model_providers.mine.experimental_bearer_token|\"sk-e2e-codex\"
config.toml|tui.notifications|true"
ABSENT[codex]="config.toml|mcp_servers
auth.json|OPENAI_API_KEY"
LOCAL[codex]="config.toml|projects|{\"/srv\": {\"trust_level\": \"trusted\"}}"
COMMENT[codex]="config.toml|# B's Codex"
EDIT[codex]="config.toml|model_reasoning_effort|\"low\""
GONE[codex]="config.toml|tui|config.toml:tui"
SECRET[codex]=sk-e2e-codex
run_codex() {
	timeout 60 codex doctor 2>&1 | grep -E 'config.toml parse +ok' >/dev/null || return 1
	timeout 90 codex exec --skip-git-repo-check hi >/dev/null 2>&1 || true
}

# --- Gemini CLI --------------------------------------------------------------
seed_gemini-cli() {
	cat >"$HOME/.gemini/settings.json" <<EOF
{
  // A's Gemini
  "security": { "auth": { "selectedType": "gemini-api-key" }, "folderTrust": { "enabled": true } },
  "model": { "name": "gem-mine" },
  "ui": { "theme": "Dracula" },
  "mcpServers": { "e2e": { "command": "echo" } }
}
EOF
	printf '# A'"'"'s Gemini env\nGEMINI_API_KEY=sk-e2e-gemini\nGOOGLE_GEMINI_BASE_URL=%s\n' "$M" >"$HOME/.gemini/.env"
	echo '{"access_token":"ya29.e2e-credential"}' >"$HOME/.gemini/oauth_creds.json"
}
default_gemini-cli() {
	# A fresh install's own theme: the pouch's wins at the first sync.
	printf '{\n  // B'"'"'s Gemini\n  "hasSeenIdeIntegrationNudge": true,\n  "ui": { "theme": "Default Light" }\n}\n' >"$HOME/.gemini/settings.json"
}
CHECK[gemini-cli]="settings.json|security.auth.selectedType|\"gemini-api-key\"
settings.json|model.name|\"gem-mine\"
settings.json|ui.theme|\"Dracula\"
.env|GEMINI_API_KEY|\"sk-e2e-gemini\"
.env|GOOGLE_GEMINI_BASE_URL|\"$M\""
ABSENT[gemini-cli]="settings.json|security.folderTrust
settings.json|mcpServers
oauth_creds.json|access_token"
LOCAL[gemini-cli]="settings.json|hasSeenIdeIntegrationNudge|true"
COMMENT[gemini-cli]="settings.json|// B's Gemini"
EDIT[gemini-cli]="settings.json|model.name|\"gem-mine-2\""
GONE[gemini-cli]="settings.json|ui|settings.json:ui"
SECRET[gemini-cli]=sk-e2e-gemini
run_gemini-cli() { timeout 120 gemini -p hi --skip-trust >/dev/null 2>&1 || true; }

# --- Qwen Code ---------------------------------------------------------------
seed_qwen-code() {
	cat >"$HOME/.qwen/settings.json" <<EOF
{
  // A's Qwen
  "modelProviders": {
    "openai": [
      { "id": "m1", "name": "M1", "baseUrl": "$M/v1", "envKey": "MINE_QWEN_KEY" },
      { "id": "m2", "name": "M2", "baseUrl": "$M/v1", "envKey": "MINE_QWEN_KEY" }
    ]
  },
  "env": { "MINE_QWEN_KEY": "sk-e2e-qwen" },
  "security": { "auth": { "selectedType": "openai" }, "folderTrust": { "enabled": true } },
  "model": { "name": "m1" },
  "ui": { "theme": "GitHub" },
  "mcpServers": { "e2e": { "command": "echo" } }
}
EOF
}
default_qwen-code() {
	printf '{\n  // B'"'"'s Qwen\n  "ide": { "enabled": false }\n}\n' >"$HOME/.qwen/settings.json"
}
CHECK[qwen-code]="settings.json|modelProviders.openai.0.baseUrl|\"$M/v1\"
settings.json|env.MINE_QWEN_KEY|\"sk-e2e-qwen\"
settings.json|security.auth.selectedType|\"openai\"
settings.json|model.name|\"m1\"
settings.json|ui.theme|\"GitHub\""
ABSENT[qwen-code]="settings.json|security.folderTrust
settings.json|mcpServers"
LOCAL[qwen-code]="settings.json|ide.enabled|false"
COMMENT[qwen-code]="settings.json|// B's Qwen"
EDIT[qwen-code]="settings.json|model.name|\"m2\""
GONE[qwen-code]="settings.json|ui|settings.json:ui"
SECRET[qwen-code]=sk-e2e-qwen
run_qwen-code() { timeout 120 qwen -p hi >/dev/null 2>&1 || true; }

# --- Copilot CLI -------------------------------------------------------------
# Copilot's own provider (BYOK) is only environment variables; its settings
# hold the model and the UI. Themes it doesn't know are dropped silently.
seed_copilot() {
	cat >"$HOME/.copilot/settings.json" <<EOF
{
  "model": "gpt-5.4",
  "theme": "dim",
  "banner": "never",
  "trustedFolders": ["/root/work"],
  "disableAllHooks": true
}
EOF
}
default_copilot() {
	printf '{\n  "trustedFolders": ["/srv"],\n  "banner": "always"\n}\n' >"$HOME/.copilot/settings.json"
}
CHECK[copilot]="settings.json|model|\"gpt-5.4\"
settings.json|theme|\"dim\"
settings.json|banner|\"never\""
ABSENT[copilot]="settings.json|disableAllHooks"
LOCAL[copilot]="settings.json|trustedFolders|[\"/srv\"]"
EDIT[copilot]="settings.json|model|\"gpt-5.5\""
GONE[copilot]="settings.json|banner|settings.json:banner"
SECRET[copilot]=
# BYOK ignores the model setting (it needs COPILOT_MODEL), so the check is
# Copilot's own reading of settings.json.
run_copilot() {
	timeout 60 copilot config --list >/tmp/copilot-config 2>&1 &&
		grep -qx 'model=gpt-5.4' /tmp/copilot-config && grep -qx 'theme=dim' /tmp/copilot-config
}

# --- Crush -------------------------------------------------------------------
# crush.json is plain JSON: no comment to keep.
seed_crush() {
	cat >"$HOME/.config/crush/crush.json" <<EOF
{
  "\$schema": "https://charm.land/crush.json",
  "providers": {
    "mine": {
      "type": "openai-compat",
      "base_url": "$M/v1",
      "api_key": "sk-e2e-crush",
      "models": [
        { "id": "m1", "name": "M1", "context_window": 100000, "default_max_tokens": 1000 },
        { "id": "m2", "name": "M2", "context_window": 100000, "default_max_tokens": 1000 }
      ]
    }
  },
  "models": {
    "large": { "model": "m1", "provider": "mine" },
    "small": { "model": "m1", "provider": "mine" }
  },
  "options": { "tui": { "compact_mode": true }, "data_directory": "/root/a-crush-data" },
  "permissions": { "allowed_tools": ["view"] },
  "mcp": { "e2e": { "type": "stdio", "command": "echo" } },
  "lsp": { "go": { "command": "gopls" } }
}
EOF
}
default_crush() {
	printf '{\n  "lsp": { "rust": { "command": "rust-analyzer" } },\n  "options": { "data_directory": "/root/b-crush-data" }\n}\n' >"$HOME/.config/crush/crush.json"
}
CHECK[crush]="crush.json|providers.mine.base_url|\"$M/v1\"
crush.json|providers.mine.api_key|\"sk-e2e-crush\"
crush.json|models.large.model|\"m1\"
crush.json|options.tui.compact_mode|true
crush.json|permissions.allowed_tools|[\"view\"]"
ABSENT[crush]="crush.json|mcp"
LOCAL[crush]="crush.json|lsp|{\"rust\": {\"command\": \"rust-analyzer\"}}
crush.json|options.data_directory|\"/root/b-crush-data\""
EDIT[crush]="crush.json|models.large.model|\"m2\""
GONE[crush]="crush.json|permissions|crush.json:permissions"
SECRET[crush]=sk-e2e-crush
run_crush() {
	timeout 60 crush models 2>/dev/null | grep -q '^mine/m1$' || return 1
	timeout 90 crush run hi >/dev/null 2>&1 || true
}

# --- Factory Droid -----------------------------------------------------------
seed_factory() {
	cat >"$HOME/.factory/settings.json" <<EOF
{
  // A's Droid
  "model": "custom:Mine-M1-0",
  "reasoningEffort": "high",
  "customModels": [
    {
      "model": "m1",
      "displayName": "Mine M1",
      "baseUrl": "$M/v1",
      "apiKey": "sk-e2e-droid",
      "provider": "generic-chat-completion-api",
      "maxOutputTokens": 1000
    }
  ],
  "sessionDefaultSettings": { "autonomyLevel": "medium" },
  "enabledPlugins": { "e2e@market": true }
}
EOF
}
default_factory() {
	printf '{\n  // B'"'"'s Droid\n  "includeCoAuthoredByDroid": false\n}\n' >"$HOME/.factory/settings.json"
}
CHECK[factory]="settings.json|model|\"custom:Mine-M1-0\"
settings.json|reasoningEffort|\"high\"
settings.json|customModels.0.baseUrl|\"$M/v1\"
settings.json|customModels.0.apiKey|\"sk-e2e-droid\"
settings.json|sessionDefaultSettings.autonomyLevel|\"medium\""
ABSENT[factory]="settings.json|enabledPlugins"
LOCAL[factory]="settings.json|includeCoAuthoredByDroid|false"
COMMENT[factory]="settings.json|// B's Droid"
EDIT[factory]="settings.json|reasoningEffort|\"low\""
GONE[factory]="settings.json|sessionDefaultSettings|settings.json:sessionDefaultSettings"
SECRET[factory]=sk-e2e-droid
run_factory() {
	timeout 60 droid doctor 2>&1 | grep -q 'No configuration issues found' || return 1
	timeout 90 droid exec hi >/dev/null 2>&1 || true
}
