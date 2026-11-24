# Local development environment

Two ways to run PostgreSQL 18 and Redis 8 locally. Both bind to `127.0.0.1` only and use passwords.

## A. Native (no Docker, no sudo) — recommended on this team's Macs

Requires: Xcode Command Line Tools (macOS) or `build-essential` (Linux), `curl`, `openssl`, Rust (`rustup`).

```bash
make -C deploy/dev install   # once: builds PostgreSQL + Redis into ~/.local/builtpanel-dev (10-30 min)
make -C deploy/dev up        # start services, create role/db and secret files
make -C deploy/dev migrate   # apply migrations
make -C deploy/dev api       # run the API on http://127.0.0.1:8080
```

Then:

- `http://127.0.0.1:8080/api/v1/public/ready` shows database and Redis status.
- `http://127.0.0.1:8080/api/docs` is the API viewer (debug builds only).

Secrets live in `~/.local/builtpanel-dev/secrets/` (mode 0600). They are never printed by `devenv.sh env`; the app reads them through `*_FILE` settings.

Other targets: `make -C deploy/dev help`.

## B. Docker Compose (alternative)

```bash
cp deploy/dev/.env.example deploy/dev/.env   # set strong passwords
docker compose -f deploy/dev/compose.yml --env-file deploy/dev/.env up -d
```

PowerDNS (the DNS server for hosted domains) starts with the same command. Its API
is `http://127.0.0.1:8053` (key: `PDNS_API_KEY`), its DNS port `127.0.0.1:5300`.
`deploy/dev/powerdns-check.sh` creates, reads and deletes a zone through the API.

Then create `0600` files containing `postgres://builtpanel:<PG_PASSWORD>@127.0.0.1:5432/builtpanel_dev` and `redis://:<REDIS_PASSWORD>@127.0.0.1:6379/0`, and point `BUILTPANEL_DATABASE__URL_FILE` / `BUILTPANEL_REDIS__URL_FILE` at them. Mailpit's web UI is at `http://127.0.0.1:8025`.

## Tests

- `make -C deploy/dev test-unit` — no services needed.
- `make -C deploy/dev test` — also runs PostgreSQL and Redis integration tests (uses the superuser for per-test databases and Redis logical DB 15).

## After changing code

- API shape changed → `make -C deploy/dev openapi` and commit `openapi.json`.
- SQL in `sqlx::query!` changed → `make -C deploy/dev sqlx-prepare` and commit `.sqlx/`.

## Building and testing on a remote box

When the workstation is slow or busy, run the checks on a Linux build box
instead. `deploy/dev/remote.sh` syncs the current worktree there and runs:

```
deploy/dev/remote.sh rust          # fmt, clippy (default + otel), tests, cargo-deny
deploy/dev/remote.sh rust test     # only one of: lint | test | deny
deploy/dev/remote.sh web           # pnpm install, lint, typecheck, test, build
deploy/dev/remote.sh web e2e       # Playwright
deploy/dev/remote.sh status        # versions, memory and disk of the box
```

The box address and SSH key live outside the repository in
`~/.config/builtpanel/remote.env`:

```
BUILTPANEL_REMOTE_HOST=dev@<host>
BUILTPANEL_REMOTE_KEY=/path/to/private/key
```

Each worktree is synced to its own `~/work/<worktree name>` directory, so
agents working in parallel do not interfere. The box needs rustup, cargo-deny,
PostgreSQL 18, Redis, Node and pnpm, plus `~/.builtpanel-dev/secrets/database_url`
(a superuser, for per-test databases) and `redis_test_url`, both mode 0600.

On a small box, keep the shared target directory from filling the disk by
putting this in the build user's `~/.cargo/config.toml`:

```
[build]
incremental = false
target-dir = "/home/<user>/target-rust"

[profile.dev]
debug = "line-tables-only"
```

If the disk still fills up, delete `~/target-rust`; the next run rebuilds it.
