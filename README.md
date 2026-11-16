# agentic-rust-api

A highly opinionated template for building Rust APIs with AI coding agents.

Agents follow rules they can check far more reliably than rules they have to
remember. So every architecture rule in this template is a check that fails:

- clippy and rustc lints
- ast-grep rules
- cargo-deny
- a dependency layering check
- a comment checker
- static SQL checks
- catalog checks against a real Postgres

Each rule has a fixture that proves it fires. `make validate` runs all of it
and is the only gate. The agent gets fast, precise feedback and has no way to
argue its way around a rule.

## Quick start

You need:

- rustup (the toolchain is pinned in `rust-toolchain.toml`)
- Docker
- `jq`
- `uv`
- `python3`

```bash
git clone https://github.com/mars-mx/agentic-rust-api.git my-api
cd my-api
make validate
```

The first run installs pinned versions of ast-grep, cargo-deny and sqlx-cli
into `.tools/`, so it takes a few minutes. Later runs are incremental.

Then point your agent at the repo. `AGENTS.md` holds the working rules, and
`CLAUDE.md` imports it. The hooks in `.claude/settings.json` work in Claude
Code:

- Edits to rule files and Markdown are blocked.
- Any comment written into Rust code is rejected.

## What `make validate` checks

| Step | What fails |
| --- | --- |
| `fmt` | unformatted code |
| `deps` | a crate that depends on something outside its layer's allowlist in `checks/allowed-deps.json`, a crate without `[lints] workspace = true`, crate-level lint config |
| `ast` | the ast-grep rules in `.ast-grep/rules`: inline SQL, routes outside the router or not behind auth, a second `IntoResponse`, config defaults, untyped secrets, float money, `#[instrument]` without `skip_all`, repository traits, transactions in `db`, and more |
| `comments` | any comment, doc comment, `#[doc]`, `#[allow]`, `#[expect]` or `#[warn]` in Rust code |
| `deny` | advisories, licenses outside the allowlist, openssl or native-tls, other Postgres drivers, ORMs, wildcard versions |
| `clippy` | pedantic clippy plus a strict deny list: no `unwrap`, `panic`, `as` casts, `std::env::var`, `sqlx::query`, `Uuid::new_v4` and more |
| `test` | unit tests: auth, Problem Details, the router, the permission catalog, config |
| `problem-details` | an OpenAPI operation with an error response that is not RFC 9457 `application/problem+json` |
| `migrations` | a migration object outside its context schema, a table without enabled and forced RLS, a policy that is not one of the five classes, an unsafe view or `SECURITY DEFINER` function, or `app.*` settings set anywhere except `scope.sql` |
| `fixtures` | a lint, ast-grep rule, layer rule or deny ban that no longer fires on its fixture |
| `hooks` | the agent hook no longer blocks what it should |
| `db` | runs against a throwaway `postgres:18`: `migrate` and its catalog checks, `cargo sqlx prepare --check`, the two-organization RLS tests, and a broken database that every catalog check must reject |

## Layout

```
crates/
  domain/     types and validation, no I/O
  db/         one async fn per query, SQL in queries/, migrations in migrations/
  app/        use cases, one transaction each
  api/        axum + utoipa: router, auth, permissions, Problem Details
  config/     typed config from environment variables only
  telemetry/  JSON tracing
bins/
  api-server/ the HTTP service, with a Dockerfile
  migrate/    applies migrations, then checks the catalog
xtask/        new-context, check-migrations, check-problem-details
guard/        the comment checker
checks/       check scripts and their fixtures
```

## The data model

Every bounded context gets its own Postgres schema and a module in `domain`,
`db`, `app` and `api`. Create one with:

```bash
cargo run -p xtask -- new-context billing
```

Postgres row level security does the tenant isolation:

- **Group roles.** `migrate` creates three NOLOGIN roles. `app_api` is the API.
  `app_view` owns the gate views and gate functions. `app_admin` is for
  back-office access. Your login roles become members of these.
- **Request scope.** Each transaction sets `app.organization_id` and
  `app.user_id` with a transaction-local `set_config` in
  `crates/db/queries/session/scope.sql`. Nothing else may set them.
- **Policy classes.** A policy is one of `org_scope`, `parent_scope`,
  `shared_read`, `admin_access` or `view_read`, and each class has exactly one
  allowed form.
- **Gates.** Some tables mix shared rows (`organization_id IS NULL`) with
  private ones. The API reads them through a gate view or a `SECURITY DEFINER`
  gate function owned by `app_view`. A gate function also keeps full-text `@@`
  queries on their GIN index, which RLS would otherwise stop.

The `catalog` context is a small worked example. It has items, shared and
private, with a gate view, a gate search function and three routes. The tests
in `crates/app/tests/two_organizations.rs` prove three things:

- An organization sees its own items and the shared ones, never another
  organization's.
- It cannot write rows for another organization.
- A query without a scope fails instead of reading rows.

`catalog` is only an example; no check depends on it. The database fixtures
in `checks/db/` bring their own schema. Keep it as a reference until your first
context works, then remove it:

1. Delete `crates/domain/src/catalog.rs`, `crates/db/src/catalog.rs`,
   `crates/app/src/catalog.rs` and `crates/api/src/catalog.rs`, and the
   `pub mod catalog;` line in each crate's `lib.rs`.
2. Delete `crates/db/queries/catalog/`, `crates/db/migrations/0001_catalog.sql`
   and `crates/app/tests/`. Keep `crates/db/migrations/.gitkeep`.
3. In `crates/api/src/router.rs`, delete the two catalog `.routes(...)` lines
   and `use crate::catalog;`. The `/api/v1/me` route stays, so the auth layer
   always has a route behind it.
4. Run `cargo fmt --all`, `make sqlx-prepare`, then `make validate`.

Your own context then needs its own two-organization test.

The organization id is the identity provider's organization id, stored as
`text`. That keeps tokens and rows in one id space, so no mapping table is
needed.

## Auth

Auth is [Clerk](https://clerk.com) through `clerk-rs`, and that is a deliberate
opinion:

- It refetches the JWKS on an unknown `kid`, rate-limited, and never lets the
  key cache expire, so key rotation needs no restart.
- `crates/api/src/auth.rs` checks `iss`, `azp` and `sts` itself.
- It turns the session token into an `Identity` with the active organization.

Everything else depends only on `Identity` and `require_auth`. To use another
provider, replace `auth.rs`, keep those two names, and update the `api` entry
in `checks/allowed-deps.json`.

## Running

Everything is configured through environment variables, and none has a default.

| Variable | Used by |
| --- | --- |
| `DATABASE_URL` | `migrate`, `api-server` |
| `LOG_LEVEL` | `migrate`, `api-server` (a tracing filter, e.g. `info`) |
| `LISTEN_ADDR` | `api-server`, e.g. `0.0.0.0:8080` |
| `CLERK_SECRET_KEY` | `api-server` |
| `CLERK_FRONTEND_API_URL` | `api-server`, the token issuer |
| `CLERK_AUTHORIZED_PARTIES` | `api-server`, comma-separated origins allowed in `azp` |

Deploy order:

1. Run `migrate` as a superuser, or as a role that has `CREATEROLE` on the
   first run and may `SET ROLE app_view`. It needs the second because it hands
   the gate views and functions to `app_view`. The checks here only exercise
   the superuser path. If a catalog check fails, `migrate` exits non-zero, and
   the new version must not start.
2. Start `api-server` with a login role that is a member of `app_api`. Never
   use the owner or a superuser: both bypass row level security.

```bash
docker build -f bins/api-server/Dockerfile -t api-server .
```

## sqlx offline mode

Queries are checked at compile time against the cached query data in `.sqlx/`,
and `SQLX_OFFLINE=true` is set in `.cargo/config.toml`. So builds need no
database. After you change a query or a migration, refresh the cache:

```bash
make sqlx-prepare
```

`make validate` fails while the cache is stale.

## License

MIT
