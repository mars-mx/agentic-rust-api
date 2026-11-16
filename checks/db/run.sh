#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd -P)"
fixtures="$root/checks/db"
cd "$root"

container="$(docker run --rm --detach --env POSTGRES_PASSWORD=postgres --publish 127.0.0.1::5432 postgres:18)"
trap 'docker stop "$container" >/dev/null' EXIT

for _ in $(seq 1 60); do
  docker exec "$container" pg_isready --quiet --host 127.0.0.1 --username postgres && break
  sleep 1
done
docker exec "$container" pg_isready --quiet --host 127.0.0.1 --username postgres

port="$(docker port "$container" 5432/tcp | head -n1 | sed 's/.*://')"
export DATABASE_URL="postgres://postgres:postgres@127.0.0.1:$port/postgres"
export LOG_LEVEL=info

cargo run --quiet --locked -p migrate

# sqlx describes each query with EXPLAIN, which plans the gate views and evaluates current_setting('app.*').
describe_url="$DATABASE_URL?options=-c%20app.organization_id%3D%20-c%20app.user_id%3D"
if [ "${1:-}" = prepare ]; then
  DATABASE_URL="$describe_url" cargo sqlx prepare --workspace -- --all-targets --locked
  exit 0
fi
DATABASE_URL="$describe_url" cargo sqlx prepare --check --workspace -- --all-targets --locked
cargo test --workspace --locked -- --ignored

docker exec --interactive "$container" psql --quiet --username postgres --set ON_ERROR_STOP=1 <"$fixtures/broken.sql"
if output="$(cargo run --quiet --locked -p migrate 2>&1)"; then
  printf 'db fixture check failed: migrate passed a database that breaks every catalog rule\n%s\n' "$output" >&2
  exit 1
fi

failures=()
while IFS= read -r expected; do
  grep -qF "$expected" <<<"$output" || failures+=("missing: $expected")
done <"$fixtures/expected.txt"
while IFS= read -r clean; do
  ! grep -qF "$clean" <<<"$output" || failures+=("reported, but keeps to the rules: $clean")
done <"$fixtures/clean.txt"

if [ "${#failures[@]}" -gt 0 ]; then
  printf 'db fixture check failed:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  printf 'migrate said:\n%s\n' "$output" >&2
  exit 1
fi
printf 'db: migrations, catalog checks, sqlx cache and the database tests pass; every catalog rule fires on its fixture\n'
