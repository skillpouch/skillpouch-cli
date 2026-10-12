# SPDX-License-Identifier: AGPL-3.0-only
# What scripts/e2e-settings.sh and scripts/e2e-plugins.sh share (sourced):
# the CLI built for the computers, the API, web app and mock provider, the
# Docker computers, and the browser. The caller sets `cli`, `repo`, `work`,
# `image`, `pcs`, the ports, `origin` and `M`, and an empty `pids` array.

fixtures="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }

cleanup() {
	set +eu
	# Bash unsets WEB and WEB_PID once the browser exits.
	local web_pid="${WEB_PID:-}" web_in="${WEB[1]:-}"
	if [ -n "$web_pid" ]; then
		[ -n "$web_in" ] && { echo quit >&"$web_in"; } 2>/dev/null
		sleep 2
		kill "$web_pid" 2>/dev/null
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
		# ss, not a test connection: under WSL that can hang on a free port.
		if [ -n "$(ss -ltnH "sport = :$port")" ]; then
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
	node "$fixtures/mock-llm.mjs" "$mock_port" "$work/mock.log" &
	pids+=($!)
	for _ in $(seq 120); do
		curl -sf "http://127.0.0.1:$api_port/readyz" >/dev/null &&
			curl -sf -o /dev/null "$origin/sign-in" && return
		sleep 1
	done
	echo "the servers didn't start; see $work/api.log and $work/web.log" >&2
	exit 1
}

# Starts the computers from $image, with the CLI and the fixtures in /e2e.
start_computers() {
	log "starting ${pcs[*]}"
	for pc in "${pcs[@]}"; do
		docker rm -f "$pc" >/dev/null 2>&1 || true
		docker run -d --name "$pc" --network host -e SKILLPOUCH_API_URL="$origin" \
			-e M="$M" "$image" sleep infinity >/dev/null
		docker exec "$pc" mkdir -p /e2e
		docker cp "$work/skillpouch" "$pc:/usr/local/bin/skillpouch" >/dev/null
		for file in harnesses.sh probe.py; do
			docker cp "$fixtures/$file" "$pc:/e2e/$file" >/dev/null
		done
	done
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
		tail -n 5 "$work/sync-$1.log" >&2
		return 1
	fi
}

# The browser: commands to web.mjs, one line back for each.
start_browser() {
	coproc WEB {
		SKILLPOUCH_WEB_DIR="$repo/apps/web" SKILLPOUCH_WEB_ORIGIN="$origin" E2E_SHOTS="$work" \
			exec node "$fixtures/web.mjs" 2>"$work/browser.log"
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
