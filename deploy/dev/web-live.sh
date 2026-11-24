#!/usr/bin/env bash
# Runs the node agent's web server tests (apps/node-agent/tests/web_live.rs) against a
# real Caddy and a real site, all in throw-away containers (T05.14).
#
#   deploy/dev/web-live.sh [path-to-the-test-binary]
#   CADDY_IMAGE=bp-caddy-waf:test WAF_DIR=/etc/caddy deploy/dev/web-live.sh   (with the firewall, T11.03)
#
# Needs Docker and the test binary built with
#   cargo test -p builtpanel-node-agent --test web_live --no-run
# Caddy's admin API is published on 127.0.0.1:12019 and its HTTP port on
# 127.0.0.1:18080; both are removed again.
set -uo pipefail
NET=bp-webtest-net
CADDY=bp-webtest-caddy
BACKEND=bp-alice-web   # the name the agent's checks demand for tenant "alice"
CADDY_IMAGE="${CADDY_IMAGE:-caddy:2}"
BUSYBOX="${BUSYBOX_IMAGE:-busybox:stable}"
TEST_BIN="${1:-$(ls -t target/debug/deps/web_live-* 2>/dev/null | grep -v '\.d$' | head -1)}"
[ -x "$TEST_BIN" ] || { echo "no test binary: build it first (see the header)"; exit 2; }
cleanup() {
  docker rm -f "$CADDY" "$BACKEND" >/dev/null 2>&1 || true
  docker network rm "$NET" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
WORK="$(mktemp -d)"
trap cleanup EXIT
cleanup_first() { docker rm -f "$CADDY" "$BACKEND" >/dev/null 2>&1; docker network rm "$NET" >/dev/null 2>&1; }
cleanup_first || true

mkdir -p "$WORK/www"
echo "hello from alice" > "$WORK/www/index.html"
echo "the new page" > "$WORK/www/new.html"
# Admin API reachable from the host; HTTP on 18080 and HTTPS on 18443 (so the
# automatic HTTPS of the agent's server uses those); certificates from Caddy's own
# local CA, which is all a test with made-up names can use.
cat > "$WORK/caddy.json" <<'JSON'
{
  "admin": {"listen": "0.0.0.0:2019", "origins": ["127.0.0.1:12019", "127.0.0.1:2019"]},
  "apps": {
    "http": {"http_port": 18080, "https_port": 18443},
    "tls": {"automation": {"policies": [{"issuers": [{"module": "internal"}]}]}}
  }
}
JSON
docker network create "$NET" >/dev/null
docker run -d --name "$BACKEND" --network "$NET" -v "$WORK/www:/www:ro" "$BUSYBOX" httpd -f -p 8080 -h /www >/dev/null
# `--restart unless-stopped`: the test stops Caddy through its API, as a crash would.
docker run -d --name "$CADDY" --network "$NET" --restart unless-stopped \
  -v "$WORK/caddy.json:/etc/caddy/init.json:ro" \
  -p 127.0.0.1:12019:2019 -p 127.0.0.1:18080:18080 -p 127.0.0.1:18443:18443 \
  "$CADDY_IMAGE" caddy run --config /etc/caddy/init.json >/dev/null
for _ in $(seq 1 30); do curl -sf http://127.0.0.1:12019/config/ >/dev/null && break; sleep 1; done

BUILTPANEL_TEST_CADDY_ADMIN=http://127.0.0.1:12019 \
BUILTPANEL_TEST_CADDY_URL=http://127.0.0.1:18080 \
BUILTPANEL_TEST_CADDY_TLS_PORT=18443 \
BUILTPANEL_TEST_CADDY_CONTAINER="$CADDY" \
BUILTPANEL_TEST_WAF_DIR="${WAF_DIR:-}" \
  "$TEST_BIN" --include-ignored --nocapture --test-threads=1 2>&1 | grep -E "^test |test result|panicked|left|right|skipped|Caddy" -A3
exit "${PIPESTATUS[0]}"
