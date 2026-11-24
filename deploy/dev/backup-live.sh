#!/usr/bin/env bash
# create_backup and restore_backup for real (T09.04, T09.05), on the build box, as root (sudo):
# the privileged helper runs as root, the agent code and Docker as the login user. Run it from
# the synced tree (`deploy/dev/remote.sh sh "sudo -n bash deploy/dev/backup-live.sh"`).
set -uo pipefail
BIN=/home/dev/target-rust/debug
ROOTDIR=/srv/tenants-live; SOCK=/tmp/bp-live-helper.sock; TR=/tmp/bp-live-transfers
runuser -u dev -- bash -lc 'cd ~/work/DHPanel-claude && . ~/.cargo/env && CARGO_TARGET_DIR=~/target-rust SQLX_OFFLINE=true cargo test -q -p builtpanel-node-agent --test backup_live --no-run 2>&1 | grep -E "^error" -A10 | head -30; CARGO_TARGET_DIR=~/target-rust SQLX_OFFLINE=true cargo build -q -p builtpanel-priv-helper 2>&1 | tail -3'
TEST=$(ls -t $BIN/build/builtpanel-node-agent/*/out/backup_live-* | grep -v "\.d$" | head -1)
rm -f $SOCK; rm -rf $TR; mkdir -p $TR; chown dev:dev $TR; chmod 770 $TR
$BIN/builtpanel-priv-helper --socket $SOCK --agent-user dev --tenants-root $ROOTDIR --transfers-dir $TR > /tmp/bp-live-helper.log 2>&1 &
HP=$!
for _ in $(seq 1 30); do [ -S $SOCK ] && break; sleep 0.2; done
runuser -u dev -- env BUILTPANEL_TEST_HELPER_SOCKET=$SOCK BUILTPANEL_TEST_TENANTS_ROOT=$ROOTDIR BUILTPANEL_AGENT_TRANSFERS=$TR $TEST --include-ignored --nocapture 2>&1 | grep -E "^test |test result|panicked|left|right|skipped|error" -A6 | head -40
echo "== left in the transfer folder: $(ls $TR | wc -l)"
tail -4 /tmp/bp-live-helper.log | sed 's/^/   /'
kill $HP 2>/dev/null; sleep 0.5
find $ROOTDIR -mindepth 1 -delete 2>/dev/null; rmdir $ROOTDIR 2>/dev/null; rm -rf $SOCK $TR
