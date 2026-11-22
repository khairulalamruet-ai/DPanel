#!/usr/bin/env bash
# The tenant tests against the Kiri engine, on the build box, as root (sudo): the privileged helper runs as
# root in its identity mode (--uid-map identity: a tenant's id N is the host's id N), the agent code as the
# login user (a member of the kiri group), the containers in Kiri. Needs kiri-lite installed and running.
#   deploy/dev/remote.sh sh "sudo -n bash deploy/dev/kiri-live.sh"
set -uo pipefail
BIN=/home/dev/target-rust/debug
ROOTDIR=/srv/tenants-kiri; SOCK=/tmp/bp-kiri-helper.sock; TR=/tmp/bp-kiri-transfers
[ -S /run/kiri/docker.sock ] || { echo "kiri is not running (systemctl status kiri)"; exit 1; }
runuser -u dev -- bash -lc 'cd ~/work/DHPanel && . ~/.cargo/env && export CARGO_TARGET_DIR=~/target-rust SQLX_OFFLINE=true && cargo build -q -p builtpanel-priv-helper 2>&1 | tail -3; for t in isolation_live tenant_live; do cargo test -q -p builtpanel-node-agent --test $t --no-run 2>&1 | grep -E "^error" -A10 | head -20; done'
rm -f $SOCK; rm -rf $TR; mkdir -p $TR; chown dev:dev $TR; chmod 770 $TR
$BIN/builtpanel-priv-helper --socket $SOCK --agent-user dev --uid-map identity --tenants-root $ROOTDIR --transfers-dir $TR > /tmp/bp-kiri-helper.log 2>&1 &
HP=$!
for _ in $(seq 1 30); do [ -S $SOCK ] && break; sleep 0.2; done
for t in tenant_live isolation_live; do
  TEST=$(ls -t $BIN/build/builtpanel-node-agent/*/out/$t-* 2>/dev/null | grep -v "\.d$" | head -1)
  [ -n "$TEST" ] || TEST=$(ls -t $BIN/deps/$t-* | grep -v "\.d$" | head -1)
  echo "=== $t"
  sg kiri -c "runuser -u dev -g kiri -- env BUILTPANEL_TEST_DOCKER_SOCKET=/run/kiri/docker.sock BUILTPANEL_TEST_HELPER_SOCKET=$SOCK BUILTPANEL_TEST_TENANTS_ROOT=$ROOTDIR BUILTPANEL_AGENT_TRANSFERS=$TR $TEST --include-ignored --nocapture" 2>&1 | grep -E "^test |test result|panicked|left|right|skipped|error" -A4 | head -40
done
tail -4 /tmp/bp-kiri-helper.log | sed 's/^/   /'
kill $HP 2>/dev/null; sleep 0.5
find $ROOTDIR -mindepth 1 -delete 2>/dev/null; rmdir $ROOTDIR 2>/dev/null; rm -rf $SOCK $TR
