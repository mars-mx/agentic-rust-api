#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
fixtures="$root/checks/deps-fixtures"
work="$(mktemp -d "${TMPDIR:-/tmp}/fixtures.XXXXXX")"
trap 'rm -rf "$work"' EXIT
cp -R "$fixtures/crates" "$work/"
sed -e 's/^members = .*/members = ["crates\/*"]/' -e '/^exclude = /d' "$root/Cargo.toml" >"$work/Cargo.toml"

if output="$("$root/checks/deps.sh" "$work" 2>&1)"; then
  printf 'deps fixture check failed: checks/deps.sh passed a workspace full of violations\n' >&2
  exit 1
fi

failures=()
while IFS= read -r expected; do
  grep -qF "$expected" <<<"$output" || failures+=("missing: $expected")
done <"$fixtures/expected.txt"

if [ "${#failures[@]}" -gt 0 ]; then
  printf 'deps fixture check failed:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  printf 'checks/deps.sh said:\n%s\n' "$output" >&2
  exit 1
fi
printf 'deps fixtures: every layer violation is reported\n'
