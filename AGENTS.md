# Working in this repository

This is a Rust API workspace with strict, checked architecture rules. Every rule
is a check that fails. `make validate` is the only gate: a change is done when it
passes, and not before.

## The loop

1. Make the change.
2. Run `make validate`. It needs Docker, because it starts a throwaway
   `postgres:18` container for the database checks.
3. Read the first failing check, fix the code, run it again.

Run one step on its own while you iterate: `make clippy`, `make test`,
`make ast`, `make migrations`, `make db`. Then run `make validate`.

## Rules you cannot change

These files define the rules. Agents never edit them, not even through Bash,
sed or git. If a rule blocks you, stop and tell the user what to change and why.

- `Cargo.toml`, `clippy.toml`, `deny.toml`, `rust-toolchain.toml`,
  `sgconfig.yml`, `Makefile`, `.cargo/`
- `.ast-grep/`, `checks/`, `guard/`, `xtask/`, `bins/migrate/`
- `scripts/agent-guard.sh`, `scripts/tests/`, `.claude/settings.json`
- every Markdown file

The hooks in `.claude/settings.json` block these edits. The post-edit hook
rejects any comment in Rust code.

## Code rules

- **No comments.** No doc comments, no `#[doc]`, no `#[allow]`, `#[expect]` or
  `#[warn]`. Make the code say it, or encode the rule in a type.
- **Layers point inward:** `api -> app -> db -> domain`. Each crate may only use
  the dependencies listed for it in `checks/allowed-deps.json`.
- **domain:** types and validation, no I/O.
- **db:** one plain `pub async fn` per query. The first argument is
  `&mut PgConnection`. SQL lives in `crates/db/queries/<context>/<name>.sql`
  and runs through `query_file!`, `query_file_as!` or `query_file_scalar!`.
  No traits, no transactions.
- **app:** one use case per function. It opens exactly one transaction with
  `Database::begin`, which sets the request scope, and commits it.
- **api:** handlers return `Result<_, AppError>`. Every error answers as
  RFC 9457 Problem Details from the one `IntoResponse` in
  `crates/api/src/error.rs`.
- **Routes:** add a route only in `crates/api/src/router.rs` as
  `.routes(routes!(handler))`, before `.route_layer(... require_auth ...)`.
  Every `#[utoipa::path]` names one permission from
  `crates/api/src/permission.rs` as `x-permission` and declares its error
  responses as `application/problem+json`.
- **Config:** comes from environment variables only, with no defaults. Secrets
  are `SecretString`. Never read `std::env` directly.
- **Data types:** IDs are UUIDv7, time is UTC `OffsetDateTime`, money is
  `rust_decimal::Decimal`.

## Database rules

- A bounded context is one module in each of `domain`, `db`, `app` and `api`,
  plus a Postgres schema of the same name. Create it with
  `cargo run -p xtask -- new-context <name>`.
- A migration is `crates/db/migrations/<NNNN>_<name>.sql`. Every object is
  schema-qualified with its context. Nothing goes into `public`.
- Every table enables and forces row level security in the migration that
  creates it.
- Policies are named after their class:
  - `org_scope`, `parent_scope` and `shared_read` go to `app_api`.
  - `admin_access` goes to `app_admin`.
  - `view_read` goes to `app_view`.
- The request's organization and user reach Postgres only through
  `crates/db/queries/session/scope.sql`, as transaction-local `app.*`
  settings.
- Some tables mix shared rows (`organization_id IS NULL`) with private ones.
  The API reads them through a gate: a gate view, or a `SECURITY DEFINER` gate
  function, owned by `app_view`. For a gate function, `REVOKE EXECUTE ... FROM
  PUBLIC` and `GRANT EXECUTE ... TO app_api` come before `OWNER TO app_view`.
  `catalog` is the worked example.
- After you change a query or a migration, refresh the committed sqlx cache
  with `make sqlx-prepare`.

`make migrations` checks each rule statically. `bins/migrate` checks them again
in `pg_catalog` after it applies the migrations, and refuses to finish if one
fails.
