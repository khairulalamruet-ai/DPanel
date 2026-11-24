#!/usr/bin/env bash
# The node firewall for real (T11.06), on the build box, as root (sudo): the privileged helper
# runs as root and loads a harmless policy (documentation addresses, unused ports) into its own
# nftables table; the packet filter checks a deny-everything shape without loading it; a refused
# ruleset leaves the table as it was. The table is removed at the end.
# `deploy/dev/remote.sh sh "sudo -n bash deploy/dev/firewall-live.sh"`
set -uo pipefail
BIN=/home/dev/target-rust/debug
ROOTDIR=/srv/tenants-live; SOCK=/tmp/bp-live-helper.sock; TR=/tmp/bp-live-transfers
runuser -u dev -- bash -lc 'cd ~/work/DHPanel-claude && . ~/.cargo/env && CARGO_TARGET_DIR=~/target-rust SQLX_OFFLINE=true cargo test -q -p builtpanel-node-agent --test nodefw_live --no-run 2>&1 | grep -E "^error" -A10 | head -30; CARGO_TARGET_DIR=~/target-rust SQLX_OFFLINE=true cargo build -q -p builtpanel-priv-helper 2>&1 | tail -3'
TEST=$(ls -t $BIN/build/builtpanel-node-agent/*/out/nodefw_live-* | grep -v "\.d$" | head -1)
rm -f $SOCK; rm -rf $TR; mkdir -p $TR; chown dev:dev $TR; chmod 770 $TR
nft delete table inet builtpanel_fw 2>/dev/null
$BIN/builtpanel-priv-helper --socket $SOCK --agent-user dev --tenants-root $ROOTDIR --transfers-dir $TR > /tmp/bp-live-helper.log 2>&1 &
HP=$!
for _ in $(seq 1 30); do [ -S $SOCK ] && break; sleep 0.2; done
echo "== as the agent, through the helper"
runuser -u dev -- env BUILTPANEL_TEST_HELPER_SOCKET=$SOCK $TEST --include-ignored --nocapture a_policy_reaches 2>&1 | grep -E "^test |test result|panicked|left|right" -A6 | head -20
echo "== the table now:"; nft list table inet builtpanel_fw | grep -E "192.0.2|elements" | head -4
echo "== as root, the packet filter's own check"
BUILTPANEL_TEST_NFT_ROOT=1 $TEST --include-ignored --nocapture the_packet_filter 2>&1 | grep -E "^test |test result|panicked|left|right" -A8 | head -20
grep -E "set_node_firewall" /tmp/bp-live-helper.log | sed 's/^/   /' | tail -6
kill $HP 2>/dev/null; sleep 0.5
nft delete table inet builtpanel_fw 2>/dev/null; echo "== table removed: $(nft list table inet builtpanel_fw 2>&1 | head -1)"
rm -rf $SOCK $TR
