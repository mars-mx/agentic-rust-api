#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
fixtures="$root/checks/lints"
lock_flag="--locked"
[ "${1:-}" = "--update-lock" ] && lock_flag=""

work="$(mktemp -d "${TMPDIR:-/tmp}/fixtures.XXXXXX")"
trap 'rm -rf "$work"' EXIT
cp "$root/clippy.toml" "$root/rust-toolchain.toml" "$work/"
[ -f "$fixtures/Cargo.lock" ] && cp "$fixtures/Cargo.lock" "$work/"
cp -R "$fixtures/pass" "$fixtures/fail" "$work/"
sed -e 's/^members = .*/members = ["pass", "fail"]/' -e '/^exclude = /d' "$root/Cargo.toml" >"$work/Cargo.toml"

(
  cd "$work"
  CARGO_TARGET_DIR="$root/target/lint-fixtures" cargo clippy --workspace --all-targets $lock_flag --keep-going \
    --message-format=json -- -D warnings >"$work/messages.json" 2>"$work/stderr.txt" || true
)
[ -z "$lock_flag" ] && cp "$work/Cargo.lock" "$fixtures/Cargo.lock"

if ! jq -e 'select(.reason == "compiler-artifact" and .target.name == "pass")' "$work/messages.json" >/dev/null; then
  cat "$work/stderr.txt" >&2
fi

found="$(jq -r '
  select(.reason == "compiler-message")
  | .message as $m
  | ($m.code.code // "" | sub("^clippy::"; "")) as $code
  | $m.spans[] | select(.is_primary) | "\(.file_name) \($code)"
' "$work/messages.json" | sort -u)"

failures=()
pass_hits="$(printf '%s\n' "$found" | grep '^pass/' || true)"
[ -z "$pass_hits" ] || failures+=("pass fixture must be clean, got: $pass_hits")
jq -e 'select(.reason == "compiler-artifact" and .target.name == "pass" and (.target.kind | index("lib")))' \
  "$work/messages.json" >/dev/null || failures+=("pass fixture did not compile")

for fixture in "$fixtures"/fail/src/*.rs "$fixtures"/fail/src/*/mod.rs; do
  rel="${fixture#"$fixtures"/}"
  name="${rel#fail/src/}"
  name="${name%%/*}"
  name="${name%.rs}"
  [ "$name" = lib ] && continue
  lint="${name%%__*}"
  printf '%s\n' "$found" | grep -qx "$rel $lint" || failures+=("$rel: expected $lint, got: $(printf '%s\n' "$found" | grep "^$rel " | cut -d' ' -f2 | tr '\n' ' ' || true)")
done

if [ "${#failures[@]}" -gt 0 ]; then
  printf 'lint fixture check failed:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi
printf 'lint fixtures: every lint fires on its fixture, the clean fixture passes\n'
