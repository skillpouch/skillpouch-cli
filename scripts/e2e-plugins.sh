#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# End-to-end test of plugins in every assistant: two Docker "computers" with
# every assistant really installed, one pouch holding two plugin folders, and
# each assistant's own CLI saying what it loaded.
#
#   scripts/e2e-plugins.sh            build, run, print the result table
#   KEEP=1 scripts/e2e-plugins.sh     leave the computers and servers up
#
# The plugins: `kit` (scripts/e2e-plugins/kit), written in the web app's
# editor, has one of each part; its MCP server and hooks note in /tmp/marks
# that they ran and found their plugin folder. ponytail
# (github.com/dietrichgebert/ponytail) comes from GitHub as a user would add
# it, so a change upstream can fail the run. Claude Code installs the plugins
# itself; every other assistant gets their parts. A runs OpenCode 1, B
# OpenCode 2. Windsurf has no Linux CLI, so it isn't checked.
#
# Needs what scripts/e2e-settings.sh needs, and GitHub reachable. Ports:
# E2E_API_PORT (18182), E2E_WEB_PORT (15182), E2E_MOCK_PORT (4998).
set -Eeuo pipefail
trap 'echo "failed at line $LINENO: $BASH_COMMAND" >&2' ERR

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cli="$(dirname "$here")"
repo="${SKILLPOUCH_REPO:-$(dirname "$cli")/skillpouch}"
api_port="${E2E_API_PORT:-18182}"
web_port="${E2E_WEB_PORT:-15182}"
mock_port="${E2E_MOCK_PORT:-4998}"
image="${E2E_IMAGE:-sp-e2e-plugins:latest}"
origin="http://localhost:$web_port"
M="http://127.0.0.1:$mock_port"
export TMPDIR="$cli/target/e2e-tmp"
mkdir -p "$TMPDIR"
work="$(mktemp -d -p "$cli/target/e2e-tmp" sp-e2e-plugins.XXXXXX)"
pcs=(sp-e2e-pa sp-e2e-pb)
pids=()
source "$here/e2e-settings/harnesses.sh"
source "$here/e2e-settings/stack.sh"
trap cleanup EXIT

{
build_cli
start_servers
log "building $image"
docker build -q -t sp-e2e-settings:latest "$here/e2e-settings" >/dev/null
docker build -q -t "$image" "$here/e2e-plugins" >/dev/null
start_computers
docker exec sp-e2e-pb ln -sf /opt/opencode2/node_modules/.bin/opencode /usr/local/bin/opencode
start_browser
read -r email pouch <<<"$(ask account)"
log "account $email, pouch $pouch"

log "the plugins: kit written in the web app, ponytail from GitHub"
python3 - "$here/e2e-plugins/kit" "$work/kit.json" <<'EOF'
import json, os, sys
root, out = sys.argv[1:]
files = {}
for folder, _, names in os.walk(root):
    for name in names:
        path = os.path.join(folder, name)
        files[os.path.relpath(path, root)] = open(path).read()
json.dump(files, open(out, 'w'))
EOF
answer="$(ask write-plugin kit "$work/kit.json")"
[ "$answer" = ok ] || { echo "adding kit: $answer" >&2; exit 1; }
answer="$(ask github-plugin dietrichgebert/ponytail)"
[ "$answer" = ok ] || { echo "adding ponytail: $answer" >&2; exit 1; }

log "A and B: every assistant set up, then the first syncs"
for pc in "${pcs[@]}"; do
	for id in "${IDS[@]}"; do on "$pc" "mkdir -p \$HOME/${ROOT[$id]} && seed_$id"; done
	on "$pc" 'mkdir -p $HOME/.cursor $HOME/.gemini/antigravity-cli'
done
connect sp-e2e-pa fill
sync_on sp-e2e-pa
connect sp-e2e-pb use_pouch
sync_on sp-e2e-pb
sync_on sp-e2e-pa

fail() { echo "FAIL: $*"; }
looks=(claude-code factory codex opencode gemini-cli qwen-code copilot crush cursor antigravity)
parts=(skill agent command mcp hook)
for pc in "${pcs[@]}"; do
	log "$pc: what each assistant loaded"
	docker cp "$here/e2e-plugins/checks.sh" "$pc:/e2e/checks.sh" >/dev/null
	# ponytail's plugin parts came, and its copies for other assistants didn't.
	ponytail=/root/.skillpouch/current/plugins/ponytail
	for path in .claude-plugin/plugin.json skills/ponytail/SKILL.md hooks/ponytail-activate.js; do
		on "$pc" "[ -f $ponytail/$path ]" || fail "$pc: ponytail came without $path"
	done
	for path in .cursor .opencode .windsurf docs; do
		on "$pc" "[ ! -e $ponytail/$path ]" || fail "$pc: ponytail brought $path"
	done
	# ponytail's own SessionStart hook, as Codex would run it.
	on "$pc" "python3 -c \"import json; print(next(h['command'] for e in json.load(open('/root/.codex/hooks.json'))['hooks']['SessionStart'] for h in e['hooks'] if 'ponytail-activate.js' in h['command']))\" >/tmp/ponytail-hook" &&
		on "$pc" 'echo "{}" | sh /tmp/ponytail-hook' | grep -q 'PONYTAIL MODE ACTIVE' ||
		fail "$pc: ponytail's SessionStart hook didn't run"

	printf '%-12s' assistant
	for part in "${parts[@]}"; do printf ' %-8s' "$part"; done
	echo
	for id in "${looks[@]}"; do
		printf '%-12s' "$id"
		if ! rows="$(on "$pc" "source /e2e/checks.sh; look_$id" 2>"$work/$pc-$id.err")"; then
			echo
			fail "$pc: checking $id failed; see $work/$pc-$id.err"
			continue
		fi
		missing=() seen=()
		while read -r part result; do
			printf ' %-8s' "$result"
			seen+=("$part")
			case "$result" in
			ok | listed | -) ;;
			*) missing+=("$part") ;;
			esac
		done <<<"$rows"
		echo
		if [ "${seen[*]}" != "${parts[*]}" ]; then fail "$pc: $id answered '${seen[*]}', not '${parts[*]}'"; fi
		if [ "${#missing[@]}" -gt 0 ]; then fail "$pc: $id is missing ${missing[*]}"; fi
	done
done | tee "$work/table.txt"
if grep -q '^FAIL' "$work/table.txt"; then
	log "something didn't load; see above"
	exit 1
fi
log "every assistant with a Linux CLI loaded both plugins (Windsurf has none)"
}
