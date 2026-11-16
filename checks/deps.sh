#!/usr/bin/env bash
set -euo pipefail

checks="$(cd "$(dirname "$0")" && pwd -P)"
root="$(cd "${1:-$checks/..}" && pwd -P)"
allow="$checks/allowed-deps.json"

metadata="$(cargo metadata --format-version 1 --no-deps --manifest-path "$root/Cargo.toml")"

violations="$(
  jq -r --slurpfile allow "$allow" '
    $allow[0] as $a
    | .packages[]
    | .name as $crate
    | if ($a | has($crate)) | not then
        "\($crate): not listed in checks/allowed-deps.json. Add the crate there with the dependencies its layer may use."
      else
        .dependencies[]
        | select(.kind == null)
        | .name as $dep
        | select(($a[$crate] | any(.[]; . == $dep)) | not)
        | "\($crate) -> \($dep): this layer may not depend on \($dep). Dependencies point inward only: api -> app -> db -> domain."
      end
  ' <<<"$metadata"
)"

while IFS= read -r manifest; do
  crate_dir="$(dirname "$manifest")"
  if ! awk '/^\[lints\]/ { in_lints = 1; next } /^\[/ { in_lints = 0 } in_lints && /^workspace[[:space:]]*=[[:space:]]*true/ { ok = 1 } END { exit !ok }' "$manifest"; then
    violations+=$'\n'"${manifest#"$root"/}: missing [lints] workspace = true. Every crate inherits the workspace lints; lint levels change only in the root Cargo.toml."
  fi
  for config in clippy.toml .clippy.toml rustfmt.toml .rustfmt.toml rust-toolchain.toml; do
    if [ -e "$crate_dir/$config" ]; then
      violations+=$'\n'"${crate_dir#"$root"/}/$config: a crate-level $config replaces the workspace one instead of extending it. Change the root $config instead."
    fi
  done
done < <(jq -r '.packages[].manifest_path' <<<"$metadata")

violations="$(printf '%s\n' "$violations" | sed '/^$/d')"
if [ -n "$violations" ]; then
  printf '%s\n' "$violations" >&2
  exit 1
fi
