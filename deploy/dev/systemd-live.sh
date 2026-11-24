#!/usr/bin/env bash
# The control plane under its real systemd unit, on the build box (T19.02), as root (sudo):
# a throw-away database with the migrations, the unit from deploy/systemd, the user, the
# configuration; then it must start, answer, run without any capability, and the sandbox must
# refuse what the unit says it refuses. Everything is removed at the end.
# `deploy/dev/remote.sh sh "sudo -n bash deploy/dev/systemd-live.sh"`
set -uo pipefail
U=/home/dev; BIN=$U/target-rust/debug/builtpanel-control-api; SEC=$U/.builtpanel-dev/secrets
DB=bp_unit_$$; fail=0
ok() { echo "ok:   $*"; }; bad() { echo "FAIL: $*"; fail=1; }
BASE=$(cat $SEC/database_url); ADMIN_URL="$BASE"; UNIT_URL="${BASE%/*}/$DB"
# a throw-away database with the schema
psql "$ADMIN_URL" -qc "CREATE DATABASE $DB" || { echo "cannot make the database"; exit 1; }
runuser -u dev -- bash -lc "cd $U/work/DHPanel-claude && . ~/.cargo/env && cargo sqlx migrate run --database-url '$UNIT_URL' --source migrations >/dev/null 2>&1" && ok "schema made" || bad "migrations"
id builtpanel >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin builtpanel
install -d -m 0750 -o root -g builtpanel /etc/builtpanel /etc/builtpanel/secrets
echo "$UNIT_URL" > /etc/builtpanel/secrets/database_url
cp $SEC/redis_url /etc/builtpanel/secrets/redis_url; cp $SEC/master_key /etc/builtpanel/secrets/master_key
# The configuration loader wants secret files private to their user (0600).
chown builtpanel:builtpanel /etc/builtpanel/secrets/*; chmod 0600 /etc/builtpanel/secrets/*
cat > /etc/builtpanel/control.toml <<'T'
listen = "127.0.0.1:18080"
[database]
url_file = "/etc/builtpanel/secrets/database_url"
[redis]
url_file = "/etc/builtpanel/secrets/redis_url"
[secrets]
master_key_file = "/etc/builtpanel/secrets/master_key"
[pki]
ca_dir = "/var/lib/builtpanel/pki"
[imports]
spool_dir = "/var/lib/builtpanel/imports"
[license]
identity_dir = "/var/lib/builtpanel/license"
T
install -m 0755 $BIN /usr/local/bin/builtpanel-control-api
install -m 0644 $U/work/DHPanel-claude/deploy/systemd/builtpanel-control-api.service /etc/systemd/system/
systemctl daemon-reload; systemctl restart builtpanel-control-api
up=0; for _ in $(seq 1 30); do
  curl -fsS http://127.0.0.1:18080/api/v1/public/health >/dev/null 2>&1 && { up=1; break; }; sleep 1; done
[ $up = 1 ] && ok "it starts and answers /health under the unit" || { bad "it did not come up"; journalctl -u builtpanel-control-api -n 15 --no-pager | cut -c1-220; }
curl -fsS http://127.0.0.1:18080/api/v1/public/ready >/dev/null 2>&1 && ok "ready: the database and Redis are reachable from inside the sandbox" || bad "ready"
PID=$(systemctl show -p MainPID --value builtpanel-control-api)
[ "$(awk '/^Uid:/{print $2}' /proc/$PID/status)" != 0 ] && ok "not root (uid $(awk '/^Uid:/{print $2}' /proc/$PID/status))" || bad "runs as root"
[ "$(awk '/^CapEff:/{print $2}' /proc/$PID/status)" = 0000000000000000 ] && ok "no capabilities" || bad "capabilities: $(grep CapEff /proc/$PID/status)"
grep -q "NoNewPrivs:.1" /proc/$PID/status && ok "no new privileges" || bad "NoNewPrivs"
[ "$(awk '/^Seccomp:/{print $2}' /proc/$PID/status)" = 2 ] && ok "a system call filter is in force" || bad "seccomp"
# the sandbox refuses what it says it refuses
nsenter -t $PID -m -- sh -c 'touch /usr/local/bin/x 2>/dev/null' && bad "it can write /usr" || ok "/usr is read only to it"
nsenter -t $PID -m -- sh -c 'ls /home 2>/dev/null | grep -q .' && bad "it can see /home" || ok "/home is hidden"
nsenter -t $PID -m -- sh -c 'touch /etc/builtpanel/probe' 2>/dev/null && bad "it can write its configuration" || ok "its configuration is read only"
[ -d /var/lib/builtpanel ] && [ "$(stat -c %U /var/lib/builtpanel)" = builtpanel ] && ok "its state folder is its own" || bad "state folder"
systemd-analyze security --no-pager builtpanel-control-api.service 2>&1 | tail -2 | sed 's/^/      /'
systemctl stop builtpanel-control-api; rm -f /etc/systemd/system/builtpanel-control-api.service /usr/local/bin/builtpanel-control-api
systemctl daemon-reload; rm -rf /etc/builtpanel /var/lib/builtpanel
psql "$ADMIN_URL" -qc "DROP DATABASE $DB" >/dev/null 2>&1
[ $fail = 0 ] && echo "ALL OK" || echo "SOME FAILED"
exit $fail
