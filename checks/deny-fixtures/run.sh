#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
fixtures="$root/checks/deny-fixtures"
work="$(mktemp -d "${TMPDIR:-/tmp}/fixtures.XXXXXX")"
trap 'rm -rf "$work"' EXIT
cp -R "$fixtures/crates" "$root/deny.toml" "$work/"
sed -e 's/^members = .*/members = ["crates\/*"]/' -e '/^exclude = /d' "$root/Cargo.toml" >"$work/Cargo.toml"

if output="$(cd "$work" && cargo deny --format json check bans 2>&1)"; then
  printf 'deny fixture check failed: cargo deny passed a crate full of banned dependencies\n' >&2
  exit 1
fi

failures=()
while IFS= read -r crate; do
  jq -e --arg crate "$crate" 'select(.type == "diagnostic" and .fields.code == "banned" and (.fields.graphs[]?.Krate.name == $crate))' \
    <<<"$output" >/dev/null 2>&1 || failures+=("$crate: not reported as banned")
done <"$fixtures/banned.txt"
jq -e 'select(.type == "diagnostic" and .fields.code == "wildcard")' <<<"$output" >/dev/null 2>&1 ||
  failures+=("itoa = \"*\": wildcard version not reported")

if [ "${#failures[@]}" -gt 0 ]; then
  printf 'deny fixture check failed:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi
printf 'deny fixtures: every banned crate and the wildcard version are reported\n'
