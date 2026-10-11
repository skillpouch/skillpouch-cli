#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# End-to-end test of assistant settings sync: three Docker "computers" with
# every supported assistant really installed, one account and pouch, the real
# API and web app, and each assistant's own CLI reading what arrived.
#
#   scripts/e2e-settings.sh            build, run, print the result table
#   KEEP=1 scripts/e2e-settings.sh     leave the computers and servers up
#
# A runs OpenCode 1, B OpenCode 2 (so OpenCode settings are translated both
# ways), C joins last with no assistant set up yet. Custom providers point at
# a mock provider (scripts/e2e-settings/mock-llm.mjs) that records the key
# and model each assistant sends: proof that the CLI accepted the settings.
#
# Needs Docker and the skillpouch repository next to this one (or
# SKILLPOUCH_REPO) with `pnpm install` done, Playwright's Chromium and the
# test Postgres (`skillpouch-pg-test`, see its AGENTS.md). Ports:
# E2E_API_PORT (18181), E2E_WEB_PORT (15181), E2E_MOCK_PORT (4999). Logs and
# temporary files go to target/e2e-tmp.
set -Eeuo pipefail
trap 'echo "failed at line $LINENO: $BASH_COMMAND" >&2' ERR

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cli="$(dirname "$here")"
repo="${SKILLPOUCH_REPO:-$(dirname "$cli")/skillpouch}"
api_port="${E2E_API_PORT:-18181}"
web_port="${E2E_WEB_PORT:-15181}"
mock_port="${E2E_MOCK_PORT:-4999}"
image="${E2E_IMAGE:-sp-e2e-settings:latest}"
origin="http://localhost:$web_port"
M="http://127.0.0.1:$mock_port"
# Everything on this side stays in the checkout (git-ignored), never in /tmp:
# the API's blobs, the browser and the build tools use TMPDIR too.
export TMPDIR="$cli/target/e2e-tmp"
mkdir -p "$TMPDIR"
work="$(mktemp -d -p "$cli/target/e2e-tmp" sp-e2e-settings.XXXXXX)"
pcs=(sp-e2e-a sp-e2e-b sp-e2e-c)
pids=()
source "$here/e2e-settings/harnesses.sh"

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }

cleanup() {
	set +e
	if [ -n "${WEB_PID:-}" ]; then
		{ echo quit >&"${WEB[1]}"; } 2>/dev/null
		sleep 2
		kill "$WEB_PID" 2>/dev/null
	fi
	if [ "${KEEP:-0}" = 1 ]; then
		log "kept: ${pcs[*]}, API :$api_port, web :$web_port, logs in $work"
		return
	fi
	for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null; done
	wait 2>/dev/null
	docker rm -f "${pcs[@]}" >/dev/null 2>&1
	log "logs in $work"
}
trap cleanup EXIT

# --- the CLI, the computers, the servers ------------------------------------

build_cli() {
	if [ -n "${SKILLPOUCH_BIN:-}" ]; then
		cp "$SKILLPOUCH_BIN" "$work/skillpouch"
		return
	fi
	log "building the CLI for Debian bookworm"
	local toolchain
	toolchain="$(sed -n 's/^rust-version = "\(.*\)"/\1/p' "$cli/Cargo.toml")"
	docker run --rm -v "$cli":/src:ro -v sp-e2e-cargo:/usr/local/cargo/registry \
		-v sp-e2e-target:/target -e CARGO_TARGET_DIR=/target -w /src \
		"rust:$toolchain-bookworm" cargo build --release --locked -p skillpouch >"$work/build.log" 2>&1
	docker run --rm -v sp-e2e-target:/target -v "$work":/out "rust:$toolchain-bookworm" \
		cp /target/release/skillpouch /out/skillpouch
}

start_servers() {
	for port in "$api_port" "$web_port" "$mock_port"; do
		if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
			echo "port $port is taken: stop what listens there or pick another (see the top of this file)" >&2
			exit 1
		fi
	done
	log "starting the API (:$api_port) and the web app (:$web_port)"
	(cd "$repo/apps/web" && E2E_API_PORT=$api_port E2E_WEB_PORT=$web_port \
		exec node --import tsx e2e/stack.ts >"$work/api.log" 2>&1) &
	pids+=($!)
	(cd "$repo/apps/web" && SKILLPOUCH_API_URL=http://127.0.0.1:$api_port WEB_PORT=$web_port \
		exec node_modules/.bin/vite --port "$web_port" --strictPort >"$work/web.log" 2>&1) &
	pids+=($!)
	node "$here/e2e-settings/mock-llm.mjs" "$mock_port" "$work/mock.log" &
	pids+=($!)
	for _ in $(seq 120); do
		curl -sf "http://127.0.0.1:$api_port/readyz" >/dev/null &&
			curl -sf -o /dev/null "$origin/sign-in" && return
		sleep 1
	done
	echo "the servers didn't start; see $work/api.log and $work/web.log" >&2
	exit 1
}

start_computers() {
	log "building $image and starting ${pcs[*]}"
	docker build -q -t "$image" "$here/e2e-settings" >/dev/null
	for pc in "${pcs[@]}"; do
		docker rm -f "$pc" >/dev/null 2>&1 || true
		docker run -d --name "$pc" --network host -e SKILLPOUCH_API_URL="$origin" \
			-e M="$M" "$image" sleep infinity >/dev/null
		docker exec "$pc" mkdir -p /e2e
		docker cp "$work/skillpouch" "$pc:/usr/local/bin/skillpouch" >/dev/null
		for file in harnesses.sh probe.py; do
			docker cp "$here/e2e-settings/$file" "$pc:/e2e/$file" >/dev/null
		done
	done
	# B runs OpenCode 2.
	docker exec sp-e2e-b ln -sf /opt/opencode2/node_modules/.bin/opencode /usr/local/bin/opencode
}

# Runs bash on a computer, with the fixtures loaded. Never with this
# script's stdin: the headless CLIs would wait on it, and loops reading
# here-strings would lose their lines. `on_stdin` passes it on.
on() {
	local pc="$1"
	shift
	docker exec -w /root "$pc" bash -c "source /e2e/harnesses.sh; $*" </dev/null
}
on_stdin() {
	local pc="$1"
	shift
	docker exec -i -w /root "$pc" bash -c "source /e2e/harnesses.sh; $*"
}

sync_on() {
	if ! on "$1" skillpouch sync >>"$work/sync-$1.log" 2>&1; then
		echo "skillpouch sync failed on $1:" >&2
		tail -5 "$work/sync-$1.log" >&2
		return 1
	fi
}

# The browser: commands to web.mjs, one line back for each.
start_browser() {
	coproc WEB {
		SKILLPOUCH_WEB_DIR="$repo/apps/web" SKILLPOUCH_WEB_ORIGIN="$origin" \
			exec node "$here/e2e-settings/web.mjs" 2>"$work/browser.log"
	}
	WEB_PID=$!
}
ask() {
	echo "$*" >&"${WEB[1]}"
	local line
	read -r -t 300 line <&"${WEB[0]}"
	echo "$line"
}

# Logs a computer in and sets it up for the pouch, as the TUI would.
connect() {
	local pc="$1" mode="$2" token
	token="$(ask token)"
	on "$pc" skillpouch login --name "$pc" "$token" >>"$work/login.log" 2>&1
	[ "$(ask connected "$pc")" = ok ]
	on_stdin "$pc" python3 - "$pouch" "$mode" <<'EOF'
import json, sys, time
pouch, mode = sys.argv[1:]
device = json.load(open('/root/.skillpouch/state/credentials.json'))['device_id']
first = {'mode': 'fill', 'assistant': None} if mode == 'fill' else {'mode': 'use_pouch'}
json.dump({'format': 1, 'device_id': device, 'pouch_id': pouch, 'first_sync': first,
           'chosen_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'done': False},
          open('/root/.skillpouch/state/setup.json', 'w'))
EOF
	# Syncs happen when the test says so; the background sync has its own step.
	on "$pc" skillpouch daemon stop >/dev/null 2>&1 || true
}

# --- checks -----------------------------------------------------------------

declare -A RESULT
SCENARIOS=(arrive cli edit delete include folder reinstall late select translate)
variant() { [ "$2" = opencode ] && { [ "$1" = sp-e2e-b ] && echo opencode@2 || echo opencode@1; } || echo "$2"; }
fixture() { local name="$1[$2]"; echo "${!name:-}"; }

pass() { [ -z "${RESULT[$1|$2]:-}" ] && RESULT[$1|$2]=ok || true; }
fail() {
	RESULT[$1|$2]="FAIL"
	echo "  ✗ $1 $2: $3" | tee -a "$work/failures.log"
}

# Every `file|path|json` line holds on the computer.
holds() {
	local pc="$1" id="$2" lines="$3" file path json
	while IFS='|' read -r file path json; do
		[ -n "$file" ] || continue
		on "$pc" python3 /e2e/probe.py eq "\$HOME/${ROOT[$id]}/$file" "'$path'" "'$json'" ||
			echo "$file:$path is $(on "$pc" python3 /e2e/probe.py get "\$HOME/${ROOT[$id]}/$file" "'$path'" || echo absent), wanted $json"
	done <<<"$lines"
	return 0
}
absent() {
	local pc="$1" id="$2" lines="$3" file path
	while IFS='|' read -r file path; do
		[ -n "$file" ] || continue
		on "$pc" python3 /e2e/probe.py get "\$HOME/${ROOT[$id]}/$file" "'$path'" >/dev/null &&
			echo "$file:$path arrived"
	done <<<"$lines"
	return 0
}
check() {
	local scenario="$1" pc="$2" id="$3" lines="$4" problems
	problems="$(holds "$pc" "$id" "$lines")"
	[ -z "$problems" ] && pass "$id" "$scenario" || fail "$id" "$scenario" "on $pc: $(echo "$problems" | head -3 | paste -sd ';')"
}
# CHECK, with step 3's edits instead of what they changed once `edited` is set.
expected() {
	local v="$1" file path json edits
	edits="$(fixture EDIT "$v")"
	while IFS='|' read -r file path json; do
		[ -n "$file" ] || continue
		if [ -n "${edited:-}" ] && grep -q "^$file|$path|" <<<"$edits"; then continue; fi
		echo "$file|$path|$json"
	done <<<"$(fixture CHECK "$v")"
	[ -z "${edited:-}" ] || echo "$edits"
}
check_all() {
	local scenario="$1" pc="$2" id="$3" v problems
	v="$(variant "$pc" "$id")"
	problems="$(holds "$pc" "$id" "$(expected "$v")")$(absent "$pc" "$id" "$(fixture ABSENT "$v")")"
	[ -z "$problems" ] && pass "$id" "$scenario" || fail "$id" "$scenario" "on $pc: $(echo "$problems" | head -3 | paste -sd ';')"
}

# The assistant's CLI sends the synced key (and the model) to the mock.
cli_check() {
	local pc="$1" id="$2" key="$3" model="$4" scenario="${5:-cli}"
	: >"$work/mock.log"
	on "$pc" "run_$id" || { fail "$id" "$scenario" "on $pc: the CLI's own config check failed"; return; }
	[ -n "$key" ] || { pass "$id" "$scenario"; return; }
	if grep -F "\"key\":\"$key\"" "$work/mock.log" | grep -qF "\"model\":\"$model\""; then
		pass "$id" "$scenario"
	else
		fail "$id" "$scenario" "on $pc: mock saw $(grep POST "$work/mock.log" | head -1 || echo nothing), wanted key $key model $model"
	fi
}
declare -A RUN_KEY=([opencode]=sk-e2e-opencode [claude-code]=sk-e2e-claude [codex]=sk-e2e-codex
	[gemini-cli]=sk-e2e-gemini [qwen-code]=sk-e2e-qwen [copilot]=
	[crush]=sk-e2e-crush [factory]=sk-e2e-droid)
declare -A RUN_MODEL=([opencode]=m1 [claude-code]=mine-sonnet [codex]=m1 [gemini-cli]=gem-mine
	[qwen-code]=m1 [copilot]=gpt-5.4 [crush]=m1 [factory]=m1)

entry_status() { on "$1" skillpouch settings list "$2" | sed -n "s/^  $3  //p"; }
install_fresh() { on "$1" "mkdir -p \$HOME/${ROOT[$2]} && default_$2"; }
# B's OpenCode is version 2: a link, not the npm package.
uninstall() {
	if [ "$1:$2" = sp-e2e-b:opencode ]; then
		on "$1" "rm -f /usr/local/bin/opencode; rm -rf \$HOME/${ROOT[$2]}"
	else
		on "$1" "npm uninstall -g ${PKG[$2]%@*} >/dev/null 2>&1; rm -rf \$HOME/${ROOT[$2]}"
	fi
}
reinstall() {
	if [ "$1:$2" = sp-e2e-b:opencode ]; then
		on "$1" ln -sf /opt/opencode2/node_modules/.bin/opencode /usr/local/bin/opencode
	else
		on "$1" "npm install -g --offline --no-fund --no-audit ${PKG[$2]}" >>"$work/npm.log" 2>&1
	fi
	install_fresh "$1" "$2"
}

# --- the scenario -----------------------------------------------------------
# One block: bash reads all of it before running any, so editing this file
# while it runs can't derail it.
{
build_cli
start_servers
start_computers
start_browser
read -r email pouch <<<"$(ask account)"
log "account $email, pouch $pouch"

log "A: assistants with custom providers, then the first sync"
for id in "${IDS[@]}"; do on sp-e2e-a "mkdir -p \$HOME/${ROOT[$id]} && seed_$id"; done
connect sp-e2e-a fill
sync_on sp-e2e-a
log "B: fresh installs, then the first sync"
for id in "${IDS[@]}"; do install_fresh sp-e2e-b "$id"; done
connect sp-e2e-b use_pouch
sync_on sp-e2e-b
sync_on sp-e2e-a
sync_on sp-e2e-b

log "1-2. settings arrive on B; B's own values survive; the CLIs accept them"
for id in "${IDS[@]}"; do
	check_all arrive sp-e2e-b "$id"
	problems="$(holds sp-e2e-b "$id" "$(fixture LOCAL "$id")")"
	[ -z "$problems" ] || fail "$id" arrive "B's own value lost: $problems"
	IFS='|' read -r file text <<<"$(fixture COMMENT "$id")"
	if [ -n "$file" ] && ! on sp-e2e-b "grep -qF \"$text\" \$HOME/${ROOT[$id]}/$file"; then
		fail "$id" arrive "the comment in $file is gone"
	fi
done
leaks="$(on sp-e2e-a "grep -rlE 'sk-ant-oat01-e2e-credential|ya29.e2e-credential|sk-e2e-codex-auth-json|e2e-tool|opencode-e2e-plugin|echo stop' \$HOME/.skillpouch/pouches/*/settings" || true)"
[ -z "$leaks" ] || for id in "${IDS[@]}"; do fail "$id" arrive "credentials or integrations in the pouch: $leaks"; done
for id in "${IDS[@]}"; do cli_check sp-e2e-b "$id" "${RUN_KEY[$id]}" "${RUN_MODEL[$id]}"; done

log "3. an edit on B reaches A"
for id in "${IDS[@]}"; do
	while IFS='|' read -r file path json; do
		on sp-e2e-b python3 /e2e/probe.py set "\$HOME/${ROOT[$id]}/$file" "'$path'" "'$json'"
	done <<<"$(fixture EDIT "$(variant sp-e2e-b "$id")")"
done
sync_on sp-e2e-b
sync_on sp-e2e-a
for id in "${IDS[@]}"; do
	check edit sp-e2e-a "$id" "$(fixture EDIT "$(variant sp-e2e-a "$id")")"
	IFS='|' read -r file text <<<"$(fixture COMMENT "$id")"
	text="${text/B/A}"
	if [ -n "$file" ] && [ "$id" != qwen-code ] && ! on sp-e2e-a "grep -qF \"$text\" \$HOME/${ROOT[$id]}/$file"; then
		fail "$id" edit "A's comment in $file is gone"
	fi
done

edited=1

log "4. a deletion on B: excluded there, kept in the pouch and on A"
for id in "${IDS[@]}"; do
	IFS='|' read -r file path entry <<<"$(fixture GONE "$(variant sp-e2e-b "$id")")"
	on sp-e2e-b python3 /e2e/probe.py del "\$HOME/${ROOT[$id]}/$file" "'$path'"
done
sync_on sp-e2e-b
sync_on sp-e2e-a
sync_on sp-e2e-b
for id in "${IDS[@]}"; do
	IFS='|' read -r file path entry <<<"$(fixture GONE "$(variant sp-e2e-b "$id")")"
	on sp-e2e-b python3 /e2e/probe.py get "\$HOME/${ROOT[$id]}/$file" "'$path'" >/dev/null &&
		fail "$id" delete "B got $file:$path back"
	IFS='|' read -r afile apath _ <<<"$(fixture GONE "$(variant sp-e2e-a "$id")")"
	on sp-e2e-a python3 /e2e/probe.py get "\$HOME/${ROOT[$id]}/$afile" "'$apath'" >/dev/null ||
		fail "$id" delete "A lost $afile:$apath"
	status="$(entry_status sp-e2e-b "$id" "$entry")"
	[ "$status" = "excluded on this computer" ] || fail "$id" delete "B lists $entry as '$status'"
	pass "$id" delete
done

log "4b. skillpouch settings include brings it back"
for id in "${IDS[@]}"; do
	IFS='|' read -r file path entry <<<"$(fixture GONE "$(variant sp-e2e-b "$id")")"
	on sp-e2e-b skillpouch settings include "$id" "$entry" >>"$work/include.log" 2>&1 ||
		fail "$id" include "skillpouch settings include failed: $(tail -1 "$work/include.log")"
done
sync_on sp-e2e-b
for id in "${IDS[@]}"; do
	IFS='|' read -r file path entry <<<"$(fixture GONE "$(variant sp-e2e-b "$id")")"
	on sp-e2e-b python3 /e2e/probe.py get "\$HOME/${ROOT[$id]}/$file" "'$path'" >/dev/null ||
		fail "$id" include "$file:$path is still missing"
	status="$(entry_status sp-e2e-b "$id" "$entry")"
	[ "$status" = synced ] || [ "$status" = translated ] || fail "$id" include "B lists $entry as '$status'"
	pass "$id" include
done

log "5. folder removed (and the entry excluded first), sync, reinstalled, sync"
for id in "${IDS[@]}"; do
	IFS='|' read -r file path entry <<<"$(fixture GONE "$(variant sp-e2e-b "$id")")"
	on sp-e2e-b skillpouch settings exclude "$id" "$entry" >>"$work/include.log" 2>&1
	uninstall sp-e2e-b "$id"
done
sync_on sp-e2e-b
for id in "${IDS[@]}"; do reinstall sp-e2e-b "$id"; done
sync_on sp-e2e-b
for id in "${IDS[@]}"; do
	check_all folder sp-e2e-b "$id"
done

log "6. uninstalled and installed again between two syncs"
for id in "${IDS[@]}"; do
	uninstall sp-e2e-b "$id"
	reinstall sp-e2e-b "$id"
done
sync_on sp-e2e-b
for id in "${IDS[@]}"; do
	check_all reinstall sp-e2e-b "$id"
done

log "7. C: set up with no assistant, assistants installed afterwards"
connect sp-e2e-c use_pouch
sync_on sp-e2e-c
for id in "${IDS[@]}"; do install_fresh sp-e2e-c "$id"; done
sync_on sp-e2e-c
for id in "${IDS[@]}"; do
	v="$(variant sp-e2e-c "$id")"
	problems="$(holds sp-e2e-c "$id" "$(fixture EDIT "$v")")$(absent sp-e2e-c "$id" "$(fixture ABSENT "$v")")"
	IFS='|' read -r file path _ <<<"$(fixture GONE "$v")"
	on sp-e2e-c python3 /e2e/probe.py get "\$HOME/${ROOT[$id]}/$file" "'$path'" >/dev/null ||
		problems+="$file:$path missing"
	[ -z "$problems" ] && pass "$id" late || fail "$id" late "on C: $problems"
done

log "8. select --off settings, and --off <assistant> across a reinstall"
on sp-e2e-b skillpouch select --off settings >/dev/null
on sp-e2e-a python3 /e2e/probe.py set "\$HOME/.claude/settings.json" model "'\"off-test\"'"
on sp-e2e-a python3 /e2e/probe.py set "\$HOME/.codex/config.toml" model "'\"off-test\"'"
sync_on sp-e2e-a
sync_on sp-e2e-b
on sp-e2e-b python3 /e2e/probe.py eq "\$HOME/.claude/settings.json" model "'\"mine-opus\"'" &&
	on sp-e2e-b python3 /e2e/probe.py eq "\$HOME/.codex/config.toml" model "'\"m1\"'" ||
	{ fail claude-code select "settings arrived with settings off"; fail codex select "settings arrived with settings off"; }
on sp-e2e-b skillpouch select --on settings >/dev/null
on sp-e2e-b skillpouch select --off claude-code >/dev/null
uninstall sp-e2e-b claude-code
reinstall sp-e2e-b claude-code
sync_on sp-e2e-b
on sp-e2e-b python3 /e2e/probe.py get "\$HOME/.claude/settings.json" model >/dev/null &&
	fail claude-code select "settings arrived for an assistant turned off"
on sp-e2e-b skillpouch select | grep -iE 'claude' >"$work/select.txt" || true
grep -qiE 'off|skip|✗|no' "$work/select.txt" || fail claude-code select "select no longer lists Claude Code as off: $(cat "$work/select.txt")"
on sp-e2e-b python3 /e2e/probe.py eq "\$HOME/.codex/config.toml" model "'\"off-test\"'" ||
	fail codex select "--on settings didn't bring A's edit"
on sp-e2e-b skillpouch select --on claude-code >/dev/null
sync_on sp-e2e-b
check select sp-e2e-b claude-code 'settings.json|model|"off-test"'
for id in "${IDS[@]}"; do pass "$id" select; done

log "9. OpenCode 1 on A, OpenCode 2 on B: a provider edit on A"
on sp-e2e-a python3 /e2e/probe.py set "\$HOME/.config/opencode/opencode.json" provider.mine.options.apiKey "'\"sk-e2e-opencode-v1-edit\"'"
sync_on sp-e2e-a
sync_on sp-e2e-b
sync_on sp-e2e-c
check translate sp-e2e-b opencode 'opencode.json|providers.mine.settings.apiKey|"sk-e2e-opencode-v1-edit"'
cli_check sp-e2e-b opencode sk-e2e-opencode-v1-edit m2 translate
cli_check sp-e2e-a opencode sk-e2e-opencode-v1-edit m2 translate
cli_check sp-e2e-c opencode sk-e2e-opencode-v1-edit m2 translate

log "10. the background sync on A notices settings edits by itself"
declare -A DAEMON_A=([opencode]='tui.json|theme|"gruvbox"' [claude-code]='settings.json|effortLevel|"high"'
	[codex]='config.toml|model_verbosity|"low"' [gemini-cli]='.env|GEMINI_API_KEY|"sk-e2e-gemini-daemon"'
	[qwen-code]='settings.json|general.vimMode|true' [copilot]='settings.json|theme|"high-contrast"'
	[crush]='crush.json|options.tui.compact_mode|false' [factory]='settings.json|reasoningEffort|"medium"')
declare -A DAEMON_B=([opencode]='cli.json|theme.name|"gruvbox"')
on sp-e2e-a skillpouch daemon start >/dev/null 2>&1
sleep 5
# First the files only settings sync reads (nothing else would wake the
# background sync), then the rest.
for round in "opencode gemini-cli copilot" "claude-code codex qwen-code crush factory"; do
	for id in $round; do
		IFS='|' read -r file path json <<<"${DAEMON_A[$id]}"
		on sp-e2e-a python3 /e2e/probe.py set "\$HOME/${ROOT[$id]}/$file" "'$path'" "'$json'"
	done
	for _ in $(seq 20); do
		sleep 3
		sync_on sp-e2e-b
		waiting=0
		for id in $round; do
			[ -z "$(holds sp-e2e-b "$id" "${DAEMON_B[$id]:-${DAEMON_A[$id]}}")" ] || waiting=1
		done
		[ "$waiting" = 0 ] && break
	done
done
for id in "${IDS[@]}"; do check daemon sp-e2e-b "$id" "${DAEMON_B[$id]:-${DAEMON_A[$id]}}"; done
on sp-e2e-a skillpouch daemon stop >/dev/null 2>&1 || true
SCENARIOS+=(daemon)

log "web: the pouch lists the Settings items, shares blank the secrets"
items="$(ask items)"
for id in "${IDS[@]}"; do
	[[ ",$items," == *",$id,"* ]] || fail "$id" web "the web app lists: $items"
	if [ -n "${SECRET[$id]}" ]; then
		answer="$(ask share "$id" "${SECRET[$id]}")"
		[[ "$answer" == ok* ]] || fail "$id" web "share link: $answer"
	fi
	pass "$id" web
done
SCENARIOS+=(web)

# --- the table --------------------------------------------------------------

printf '\n%-12s' assistant
for s in "${SCENARIOS[@]}"; do printf ' %-9s' "$s"; done
echo
failed=0
for id in "${IDS[@]}"; do
	printf '%-12s' "$id"
	for s in "${SCENARIOS[@]}"; do
		r="${RESULT[$id|$s]:--}"
		[ "$r" = FAIL ] && failed=1
		printf ' %-9s' "$r"
	done
	echo
done
[ "$failed" = 0 ] || { echo; cat "$work/failures.log"; exit 1; }
exit 0
}
