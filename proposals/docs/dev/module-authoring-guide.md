# Writing a backend module (T03.14)

A module is a crate under `crates/modules/<name>/` plus a manifest `modules/<name>/module.toml`.
Use `crates/modules/cron` as the smallest complete example and copy its layout.

## Checklist

1. **Manifest** `modules/<name>/module.toml`: `[module]` (name, version, title in `en` and `bn`,
   category, `core`, `depends_on`), `[permissions]`, `[features]` (plan feature keys), `[limits]`
   (plan limits with `default` and `unlimited`) and `[ui]` (navigation). Unknown keys are errors.
   Every problem is reported at once at start-up.
2. **Crate** `crates/modules/<name>/Cargo.toml` named `builtpanel-mod-<name>`, with `[lints] workspace =
   true`, and add it to the workspace `members` in the root `Cargo.toml` (dependency changes go in
   their own small PR, newest stable versions).
3. **`lib.rs`** exports `MANIFEST` (`include_str!` of the manifest), an optional `SETTINGS_SCHEMA`
   (`modules/<name>/settings.schema.json`), a `State` struct (`db`, `sessions`, `settings` as
   needed, with `FromRef` for the pool), and `client_router()` / `admin_router()` / `public_router()`
   returning `Router<State>`.
4. **Paths** are full: `/api/v1/client/...`, `/api/v1/admin/...`, `/api/v1/public/...`. A handler
   checks the session, the permission (`builtpanel_rbac::Principal`), the plan feature
   (`plans::includes_feature`) and that the thing belongs to the caller (answer `404`, not `403`,
   for another account's id).
5. **Wire it** in `apps/control-api/src/modules.rs`: add the manifest to `EMBEDDED`, a match arm
   that builds `ModuleRoutes { admin, client, public }` from the state (return `none()` until the
   settings registry exists), and the name to the module list near the end of that file. Add the
   `OpenApi` of the module to `apps/control-api/src/openapi.rs`.
6. **Permissions**: new keys go in `crates/rbac/src/lib.rs` and the manifest's `[permissions]`.
7. **Database**: a migration pair `migrations/<timestamp>_<name>.up.sql` / `.down.sql`, queries in
   `crates/database/src/<name>.rs`. Queries are checked at build time: after changing SQL run
   `cargo sqlx prepare` against the dev database and commit `.sqlx/`. Several writes that belong
   together happen in one transaction; lock the row you check a limit against.
8. **Work on a node** is a typed operation, never a shell command: add a variant to
   `crates/node-protocol/src/lib.rs` (arguments are plain, validated data), a handler in
   `apps/node-agent/src/ops/`, enqueue it with `operations::enqueue` from the handler, and settle
   the result in `crates/dispatcher/src/lib.rs`. Secrets are never put into the operation by the
   handler: store them sealed and let `apps/control-api/src/secrets.rs` fill them in as it is sent.
9. **Audit and events**: write an `AuditEntry` for every change; emit events the notifications
   module can use.
10. **Errors** use `AppError` (RFC 9457 problem details) with a stable `code`; never put internal
    details or secrets in a message.
11. **Tests**: unit tests for pure code; a `tests/<name>_live.rs` that runs the router against real
    PostgreSQL and Redis (copy `cron_live.rs`), including: another account's id gives 404, a plan
    without the feature gives 403, a limit is enforced, and the audit row exists. Agent operations
    get a live test in `apps/node-agent/tests/`.
12. **Before the PR**: `deploy/dev/remote.sh rust all` (fmt, clippy `-D warnings`, tests, deny),
    regenerate `openapi.json` and flag shape changes `API-CHANGE`. `unwrap` and `expect` are denied
    outside tests.

## Rules that never bend
No secret in code, logs, commits or plaintext columns. The control plane never runs as root and
never gets a shell on a node. Only the node agent touches the Docker socket. Images come from
`deploy/stack/images.toml`, pinned by digest.
