#!/usr/bin/env bash
# Prints the next release version, from the latest `v*` tag and the commits
# since it:
#   - a commit whose message has `[major]` or `BREAKING`  → major (x+1.0.0)
#   - a commit whose message has `[minor]` or starts with `feat` → minor
#   - anything else                                       → patch
# Without a tag yet, the workspace version in Cargo.toml is the first release.
set -euo pipefail

workspace_version() {
	sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}' Cargo.toml | head -n1
}

last_tag=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
if [[ -z "$last_tag" ]]; then
	workspace_version
	exit 0
fi

messages=$(git log --format='%s%n%b' "$last_tag..HEAD")
if [[ -z "$(git rev-list "$last_tag..HEAD")" ]]; then
	echo "no commits since $last_tag" >&2
	exit 1
fi

IFS=. read -r major minor patch <<<"${last_tag#v}"
if grep -qE '\[major\]|BREAKING' <<<"$messages"; then
	echo "$((major + 1)).0.0"
elif grep -qE '\[minor\]|^feat' <<<"$messages"; then
	echo "$major.$((minor + 1)).0"
else
	echo "$major.$minor.$((patch + 1))"
fi
