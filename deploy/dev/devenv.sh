#!/usr/bin/env bash
# Built Panel native development environment (no Docker required).
#
# Builds PostgreSQL 18 and Redis 8 from official sources into a per-user
# prefix, runs them on 127.0.0.1 only with random passwords, and keeps every
# secret in 0600 files. Nothing is installed system-wide; no sudo.
#
#   devenv.sh install   download, verify and build PostgreSQL + Redis (once)
#   devenv.sh init      create data dirs, passwords, builtpanel role + database
#   devenv.sh up        start PostgreSQL and Redis (runs init if needed)
#   devenv.sh down      stop both
#   devenv.sh status    show what is running
#   devenv.sh env       print `export` lines for the app (file paths only, no secrets)
#   devenv.sh test-env  print `export` lines for integration tests (reads secrets into env)
#
# Location: $BUILTPANEL_DEV_HOME (default ~/.local/builtpanel-dev).
set -euo pipefail

PG_VERSION="${PG_VERSION:-18.6}"
REDIS_VERSION="${REDIS_VERSION:-8.10.3}"
PG_PORT="${PG_PORT:-5432}"
REDIS_PORT="${REDIS_PORT:-6379}"
HOME_DIR="${BUILTPANEL_DEV_HOME:-$HOME/.local/builtpanel-dev}"

PG_BIN="$HOME_DIR/pgsql/bin"
REDIS_BIN="$HOME_DIR/redis/bin"
SECRETS="$HOME_DIR/secrets"
PGDATA="$HOME_DIR/pgdata"
LOGS="$HOME_DIR/log"

log() { printf '[devenv] %s\n' "$*"; }
die() { printf '[devenv] error: %s\n' "$*" >&2; exit 1; }

ncpu() { getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 2; }
sha256_check() { # file.sha256 in the current directory
  if command -v sha256sum >/dev/null; then sha256sum -c "$1"; else shasum -a 256 -c "$1"; fi
}
port_in_use() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
random_secret() { openssl rand -base64 32 | tr -d '/+=\n'; }
# 32 random bytes, standard base64 (the format secrets.master_key_file expects).
ensure_master_key() {
  [ -s "$SECRETS/master_key" ] || ( umask 077; openssl rand -base64 32 >"$SECRETS/master_key" )
}

cmd_install() {
  local src="$HOME_DIR/src"
  mkdir -p "$src" "$LOGS"
  if [ -x "$PG_BIN/postgres" ]; then
    log "PostgreSQL already installed: $("$PG_BIN/postgres" --version)"
  else
    log "building PostgreSQL $PG_VERSION (this takes a while)"
    (
      cd "$src"
      curl -fsSLO "https://ftp.postgresql.org/pub/source/v$PG_VERSION/postgresql-$PG_VERSION.tar.bz2"
      curl -fsSLO "https://ftp.postgresql.org/pub/source/v$PG_VERSION/postgresql-$PG_VERSION.tar.bz2.sha256"
      sha256_check "postgresql-$PG_VERSION.tar.bz2.sha256"
      tar xjf "postgresql-$PG_VERSION.tar.bz2"
      cd "postgresql-$PG_VERSION"
      ./configure --prefix="$HOME_DIR/pgsql" --without-icu --without-readline >"$LOGS/pg-configure.log" 2>&1
      make -j"$(ncpu)" >"$LOGS/pg-make.log" 2>&1
      make install >"$LOGS/pg-install.log" 2>&1
    ) || die "PostgreSQL build failed; see $LOGS/pg-*.log"
    log "installed: $("$PG_BIN/postgres" --version)"
  fi

  if [ -x "$REDIS_BIN/redis-server" ]; then
    log "Redis already installed: $("$REDIS_BIN/redis-server" --version | cut -d' ' -f1-3)"
  else
    log "building Redis $REDIS_VERSION (core server only)"
    (
      cd "$src"
      curl -fsSLO "https://download.redis.io/releases/redis-$REDIS_VERSION.tar.gz"
      tar xzf "redis-$REDIS_VERSION.tar.gz"
      cd "redis-$REDIS_VERSION"
      # Optional Redis 8 modules (search/json/bloom/timeseries) are not used by
      # Built Panel and need newer build tools; their failure must not stop the core build.
      make -j"$(ncpu)" BUILD_WITH_MODULES=no >"$LOGS/redis-make.log" 2>&1 || true
      [ -x src/redis-server ] || exit 1
      make PREFIX="$HOME_DIR/redis" BUILD_WITH_MODULES=no install >"$LOGS/redis-install.log" 2>&1
    ) || die "Redis build failed; see $LOGS/redis-*.log"
    log "installed: $("$REDIS_BIN/redis-server" --version | cut -d' ' -f1-3)"
  fi
}

cmd_init() {
  [ -x "$PG_BIN/initdb" ] || die "PostgreSQL not installed; run: devenv.sh install"
  [ -x "$REDIS_BIN/redis-server" ] || die "Redis not installed; run: devenv.sh install"
  ( umask 077; mkdir -p "$SECRETS" "$LOGS" "$HOME_DIR/redis-data" )
  chmod 700 "$SECRETS"
  for name in pg_superuser_password pg_builtpanel_password redis_password; do
    [ -s "$SECRETS/$name" ] || ( umask 077; random_secret >"$SECRETS/$name" )
  done
  ensure_master_key

  if [ ! -f "$PGDATA/PG_VERSION" ]; then
    log "initialising PostgreSQL data directory"
    "$PG_BIN/initdb" -D "$PGDATA" -U postgres --auth-local=peer --auth-host=scram-sha-256 \
      --pwfile="$SECRETS/pg_superuser_password" --encoding=UTF8 --locale=C >"$LOGS/initdb.log" 2>&1
  fi

  ( umask 077
    cat >"$HOME_DIR/redis.conf" <<EOF
bind 127.0.0.1
port $REDIS_PORT
protected-mode yes
daemonize yes
pidfile $HOME_DIR/redis.pid
logfile $LOGS/redis.log
dir $HOME_DIR/redis-data
requirepass $(cat "$SECRETS/redis_password")
EOF
  )
}

pg_running() { "$PG_BIN/pg_ctl" -D "$PGDATA" status >/dev/null 2>&1; }
redis_running() {
  [ -f "$HOME_DIR/redis.pid" ] && kill -0 "$(cat "$HOME_DIR/redis.pid")" 2>/dev/null
}

psql_super() { # psql as postgres over TCP, password from file (never on the command line)
  PGPASSFILE="$SECRETS/pgpass" "$PG_BIN/psql" -h 127.0.0.1 -p "$PG_PORT" -U postgres -v ON_ERROR_STOP=1 -q "$@"
}

ensure_role_and_db() {
  ( umask 077; printf '127.0.0.1:%s:*:postgres:%s\n' "$PG_PORT" "$(cat "$SECRETS/pg_superuser_password")" >"$SECRETS/pgpass" )
  # Password goes through the environment, not argv, so it never shows in `ps`.
  BUILTPANEL_PW="$(cat "$SECRETS/pg_builtpanel_password")" psql_super -d postgres <<'SQL'
\getenv pw BUILTPANEL_PW
SELECT 'CREATE ROLE builtpanel LOGIN PASSWORD ' || quote_literal(:'pw')
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'builtpanel') \gexec
SELECT 'CREATE DATABASE builtpanel_dev OWNER builtpanel'
WHERE NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = 'builtpanel_dev') \gexec
SQL
  ( umask 077
    printf 'postgres://builtpanel:%s@127.0.0.1:%s/builtpanel_dev\n' "$(cat "$SECRETS/pg_builtpanel_password")" "$PG_PORT" >"$SECRETS/database_url"
    printf 'redis://:%s@127.0.0.1:%s/0\n' "$(cat "$SECRETS/redis_password")" "$REDIS_PORT" >"$SECRETS/redis_url"
  )
}

cmd_up() {
  cmd_init
  if pg_running; then
    log "PostgreSQL already running"
  else
    port_in_use "$PG_PORT" && die "port $PG_PORT is in use by another process"
    "$PG_BIN/pg_ctl" -D "$PGDATA" -l "$LOGS/postgres.log" \
      -o "-c listen_addresses=127.0.0.1 -p $PG_PORT" -w start >/dev/null
    log "PostgreSQL started on 127.0.0.1:$PG_PORT"
  fi
  if redis_running; then
    log "Redis already running"
  else
    port_in_use "$REDIS_PORT" && die "port $REDIS_PORT is in use by another process"
    "$REDIS_BIN/redis-server" "$HOME_DIR/redis.conf"
    log "Redis started on 127.0.0.1:$REDIS_PORT"
  fi
  ensure_role_and_db
  log "ready. Load app settings with: eval \"\$($0 env)\""
}

cmd_down() {
  if pg_running; then "$PG_BIN/pg_ctl" -D "$PGDATA" -m fast stop >/dev/null; log "PostgreSQL stopped"; else log "PostgreSQL not running"; fi
  if redis_running; then
    REDISCLI_AUTH="$(cat "$SECRETS/redis_password")" "$REDIS_BIN/redis-cli" -h 127.0.0.1 -p "$REDIS_PORT" --no-auth-warning shutdown >/dev/null 2>&1 || true
    log "Redis stopped"
  else
    log "Redis not running"
  fi
}

cmd_status() {
  local pg="stopped" rd="stopped"
  [ -x "$PG_BIN/pg_ctl" ] && pg_running && pg="running on 127.0.0.1:$PG_PORT"
  [ -x "$REDIS_BIN/redis-server" ] && redis_running && rd="running on 127.0.0.1:$REDIS_PORT"
  printf 'home:       %s\n' "$HOME_DIR"
  printf 'postgres:   %s (%s)\n' "$pg" "$([ -x "$PG_BIN/postgres" ] && "$PG_BIN/postgres" --version || echo 'not installed')"
  printf 'redis:      %s (%s)\n' "$rd" "$([ -x "$REDIS_BIN/redis-server" ] && "$REDIS_BIN/redis-server" --version | cut -d' ' -f3 || echo 'not installed')"
  printf 'secrets:    %s\n' "$(ls "$SECRETS" 2>/dev/null | tr '\n' ' ')"
}

cmd_env() { # file paths only; secrets stay in their 0600 files
  [ -d "$SECRETS" ] && ensure_master_key # environments created before 2FA existed
  local repo
  repo="$(cd "$(dirname "$0")/../.." && pwd)"
  cat <<EOF
export BUILTPANEL_CONFIG='$repo/deploy/dev/control.dev.toml'
export BUILTPANEL_DATABASE__URL_FILE='$SECRETS/database_url'
export BUILTPANEL_REDIS__URL_FILE='$SECRETS/redis_url'
export BUILTPANEL_SECRETS__MASTER_KEY_FILE='$SECRETS/master_key'
export BUILTPANEL_PKI__CA_DIR='$SECRETS'
export SQLX_OFFLINE=true
EOF
}

cmd_test_env() { # for `cargo test -- --include-ignored`: superuser URL (CREATEDB) and Redis db 15
  printf "export DATABASE_URL='postgres://postgres:%s@127.0.0.1:%s/postgres'\n" "$(cat "$SECRETS/pg_superuser_password")" "$PG_PORT"
  printf "export REDIS_URL='redis://:%s@127.0.0.1:%s/15'\n" "$(cat "$SECRETS/redis_password")" "$REDIS_PORT"
}

case "${1:-}" in
  install) cmd_install ;;
  init) cmd_init; log "initialised (start with: $0 up)" ;;
  up) cmd_up ;;
  down) cmd_down ;;
  status) cmd_status ;;
  env) cmd_env ;;
  test-env) cmd_test_env ;;
  *) sed -n '2,16p' "$0"; exit 2 ;;
esac
