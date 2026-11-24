#!/usr/bin/env bash
# The server tools for real (T10.15), on the build box, as root (sudo): the privileged helper
# runs the tools directly (no systemd units) with a stand-in for the reboot; the agent code runs
# as the login user. Time zone, /etc/fstab and the swap are put back afterwards.
# `deploy/dev/remote.sh sh "sudo -n bash deploy/dev/server-live.sh"`
set -uo pipefail
BIN=/home/dev/target-rust/debug
ROOTDIR=/srv/tenants-live; SOCK=/tmp/bp-live-helper.sock; TR=/tmp/bp-live-transfers; SWAP=/var/tmp/bp-test-swap
runuser -u dev -- bash -lc 'cd ~/work/DHPanel-claude && . ~/.cargo/env && CARGO_TARGET_DIR=~/target-rust SQLX_OFFLINE=true cargo test -q -p builtpanel-node-agent --test server_live --no-run 2>&1 | grep -E "^error" -A10 | head -30; CARGO_TARGET_DIR=~/target-rust SQLX_OFFLINE=true cargo build -q -p builtpanel-priv-helper 2>&1 | tail -3'
TEST=$(ls -t $BIN/build/builtpanel-node-agent/*/out/server_live-* | grep -v "\.d$" | head -1)
OLD_TZ=$(readlink /etc/localtime); cp /etc/fstab /tmp/fstab.saved
rm -f $SOCK; rm -rf $TR; mkdir -p $TR; chown dev:dev $TR; chmod 770 $TR
$BIN/builtpanel-priv-helper --socket $SOCK --agent-user dev --tenants-root $ROOTDIR --transfers-dir $TR \
  --direct-server-tools --reboot-command /bin/true --swap-file $SWAP > /tmp/bp-live-helper.log 2>&1 &
HP=$!
for _ in $(seq 1 30); do [ -S $SOCK ] && break; sleep 0.2; done
runuser -u dev -- env BUILTPANEL_TEST_HELPER_SOCKET=$SOCK BUILTPANEL_TEST_SWAP_FILE=$SWAP $TEST --include-ignored --nocapture 2>&1 | grep -E "^test |test result|panicked|left|right|skipped|error" -A6 | head -40
echo "== helper log:"; grep -E "set_timezone|set_swap|drop_caches|reboot" /tmp/bp-live-helper.log | sed 's/^/   /' | tail -12
kill $HP 2>/dev/null; sleep 0.5
swapoff $SWAP 2>/dev/null; rm -f $SWAP; cp /tmp/fstab.saved /etc/fstab
[ -n "$OLD_TZ" ] && ln -sf "$OLD_TZ" /etc/localtime
echo "== time zone now: $(readlink /etc/localtime); fstab restored: $(diff -q /etc/fstab /tmp/fstab.saved && echo yes)"
rm -rf $SOCK $TR /tmp/fstab.saved
