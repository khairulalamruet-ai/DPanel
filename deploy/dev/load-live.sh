#!/usr/bin/env bash
# Load test of the control plane on the build box (T19.05): 500 tenants in the database, 100
# people on the panel at once and 1000 operations a minute for DURATION (default 2m), with k6.
#
#   deploy/dev/remote.sh sh 'deploy/dev/load-live.sh'
#
# Needs the release binaries (`cargo build --release -p builtpanel-control-api -p builtpanel-bpctl`), k6 in
# ~/bin, PostgreSQL and Redis with the secrets of remote.sh. It makes its own database
# (`bp_load`), runs the panel on 127.0.0.1:18080 and removes both afterwards. A debug build
# on two cores is a hard case: a pass here is a pass on production hardware.
set -uo pipefail
. "$HOME/.cargo/env" 2> /dev/null || true
BIN="${CARGO_TARGET_DIR:-$HOME/target-rust}/release"
S="$HOME/.builtpanel-dev/secrets"
BASE_URL="$(cat "$S/database_url")"; BASE_URL="${BASE_URL%/*}"
WORK="$(mktemp -d)"; PORT=18080; ACCOUNTS=500
PASSWORD="load-test-password-Zq7-4471"
cleanup() {
    [ -n "${FAKE:-}" ] && kill "$FAKE" 2> /dev/null
    [ -n "${PID:-}" ] && kill "$PID" 2> /dev/null
    psql "$(cat "$S/database_url")" -qc 'DROP DATABASE IF EXISTS bp_load' > /dev/null 2>&1
    rm -rf "$WORK"
}
trap cleanup EXIT
psql "$(cat "$S/database_url")" -qc 'DROP DATABASE IF EXISTS bp_load' -c 'CREATE DATABASE bp_load' || exit 2
DB="$BASE_URL/bp_load"
umask 077
printf '%s' "$DB" > "$WORK/db"; cp "$S/redis_url" "$WORK/redis"; cp "$S/master_key" "$WORK/master"
mkdir -p "$WORK/pki"
cat > "$WORK/control.toml" <<TOML
listen = "127.0.0.1:$PORT"
[log]
format = "json"
filter = "warn"
[database]
url_file = "$WORK/db"
max_connections = 20
[redis]
url_file = "$WORK/redis"
[secrets]
master_key_file = "$WORK/master"
[pki]
ca_dir = "$WORK/pki"
TOML
export BUILTPANEL_CONFIG="$WORK/control.toml"
"$BIN/bpctl" migrate > /dev/null || exit 2
"$BIN/bpctl" ca init --dir "$WORK/pki" > /dev/null || exit 2
printf '%s\n' "$PASSWORD" | "$BIN/bpctl" admin create --username loadadmin --email loadadmin@example.com --password-stdin > /dev/null || exit 2
HASH="$(psql "$DB" -Atqc "SELECT password_hash FROM accounts WHERE username = 'loadadmin'")"
# 500 tenants: an account, a tenant, a domain, a database and a cron job each (the lists the
# people look at are not empty).
psql "$DB" -q -v hash="$HASH" -v n="$ACCOUNTS" <<'SQL' || exit 2
INSERT INTO nodes (name) VALUES ('load-node');
INSERT INTO accounts (kind, username, email, password_hash)
  SELECT 'client', 'load' || g, 'load' || g || '@example.com', :'hash' FROM generate_series(0, :n - 1) g;
INSERT INTO account_roles (account_id, role_id)
  SELECT a.id, r.id FROM accounts a, roles r WHERE a.username LIKE 'load%' AND a.username <> 'loadadmin' AND r.name = 'client';
INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
  SELECT a.id, (SELECT id FROM nodes WHERE name = 'load-node'), (SELECT id FROM plans WHERE is_default),
         a.username, 20000 + substr(a.username, 5)::int, 'active'
  FROM accounts a WHERE a.username ~ '^load[0-9]+$';
INSERT INTO domains (tenant_id, fqdn, kind) SELECT id, username || '.example', 'primary' FROM tenants WHERE username ~ '^load[0-9]+$';
INSERT INTO databases (tenant_id, name, status) SELECT id, 'db_' || username, 'ready' FROM tenants WHERE username ~ '^load[0-9]+$';
SQL
echo "tenants: $(psql "$DB" -Atqc "SELECT count(*) FROM tenants")"
"$BIN/builtpanel-control-api" > "$WORK/api.log" 2>&1 &
PID=$!
# A node that answers at once: no real node is connected, so a loop plays one, taking every
# pending operation and answering a restart as the agent does. What is measured is the
# panel's side (enqueue, wait, answer); the agent's own work is not part of this test.
(
    while true; do
        psql "$DB" -qAt -c "UPDATE operations SET status = 'dispatched', attempts = attempts + 1, dispatched_at = now() WHERE status = 'pending'" \
            -c "UPDATE operations SET status = 'succeeded', finished_at = now(), output = jsonb_build_object('type', 'container_restarted', 'name', payload->>'name', 'state', 'running') WHERE status = 'dispatched' AND kind = 'restart_container'" > /dev/null 2>&1
        sleep 0.05
    done
) &
FAKE=$!
for _ in $(seq 1 60); do curl -sf "http://127.0.0.1:$PORT/api/v1/public/ready" > /dev/null && break; sleep 0.5; done
curl -sf "http://127.0.0.1:$PORT/api/v1/public/ready" > /dev/null || { echo "the panel did not start:"; tail -20 "$WORK/api.log"; exit 3; }
# The release smoke test (T19.11) runs against this panel first.
deploy/release/smoke.sh "http://127.0.0.1:$PORT" || exit 4
"$HOME/bin/k6" run --quiet --summary-trend-stats 'avg,med,p(90),p(95),max' \
    -e BASE="http://127.0.0.1:$PORT" -e ACCOUNTS="$ACCOUNTS" -e PASSWORD="$PASSWORD" -e DURATION="${DURATION:-2m}" \
    tests/load/panel.js
rc=$?
echo "operations in the ledger: $(psql "$DB" -Atqc "SELECT string_agg(kind || '=' || n, ', ') FROM (SELECT kind, count(*) n FROM operations GROUP BY kind) t")"
echo "panel log (errors): $(grep -c "\"level\":\"ERROR\"" "$WORK/api.log")"; grep "\"level\":\"ERROR\"" "$WORK/api.log" | cut -c1-300 | sort | uniq -c | sort -rn | head -5
exit $rc
