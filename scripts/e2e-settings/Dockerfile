# SPDX-License-Identifier: AGPL-3.0-only
# One "computer" for scripts/e2e-settings.sh: every assistant with settings
# sync, installed for real at the versions the test was written against.
# OpenCode 2 lives beside OpenCode 1 in /opt/opencode2; a computer that should
# run it links it over `opencode`. npm's cache stays, so an assistant can be
# uninstalled and installed again offline.
FROM node:22-bookworm

RUN npm install -g --no-fund --no-audit \
		@anthropic-ai/claude-code@2.1.296 \
		@charmland/crush@0.98.1 \
		@github/copilot@1.0.95 \
		@google/gemini-cli@0.63.0 \
		@openai/codex@0.162.1 \
		@qwen-code/qwen-code@0.25.0 \
		opencode-ai@1.19.0 \
		droid@0.238.0 \
	&& npm install --no-fund --no-audit --prefix /opt/opencode2 @opencode/cli@2.0.26 \
	&& ln -s /opt/opencode2/node_modules/.bin/opencode /usr/local/bin/opencode2
