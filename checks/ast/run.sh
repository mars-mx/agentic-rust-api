#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
fixtures="$root/checks/ast"
work="$(mktemp -d "${TMPDIR:-/tmp}/fixtures.XXXXXX")"
trap 'rm -rf "$work"' EXIT
cp -R "$root/sgconfig.yml" "$root/.ast-grep" "$fixtures/crates" "$fixtures/bins" "$work/"

found="$( (cd "$work" && ast-grep scan --json=stream 2>/dev/null || true) | jq -r '"\(.file) \(.ruleId)"' | sort -u)"

failures=()
for rule in "$root"/.ast-grep/rules/*.yml; do
  id="$(basename "$rule" .yml)"
  file="$(cd "$fixtures" && find crates bins -name "$id.rs")"
  if [ -z "$file" ]; then
    failures+=("$id: no fixture under checks/ast")
  elif ! printf '%s\n' "$found" | grep -qx "$file $id"; then
    failures+=("$file: expected $id, got: $(printf '%s\n' "$found" | grep "^$file " | cut -d' ' -f2 | tr '\n' ' ' || true)")
  fi
done

if [ "${#failures[@]}" -gt 0 ]; then
  printf 'ast-grep fixture check failed:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi
printf 'ast-grep fixtures: every rule fires on its fixture path\n'
