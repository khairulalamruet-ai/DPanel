#!/usr/bin/env bash
# Upgrades a running panel server to a release (T19.11), run as root on that server from the
# unpacked release:
#
#   sudo ./release/deploy.sh [--bin-dir ./bin] [--web-dir ./web]
#
# Order: keep the old binaries, install the new ones, apply migrations, restart, wait until the
# panel is ready. When it does not become ready the old binaries are put back and restarted
# (migrations only add; the previous version runs on the migrated database - a release that
# breaks that rule must say so in its changelog and is not deployed with this script).
set -euo pipefail
BIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/bin"
WEB_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --bin-dir) BIN_DIR="${2:?--bin-dir needs a folder}"; shift 2 ;;
        --web-dir) WEB_DIR="${2:?--web-dir needs a folder}"; shift 2 ;;
        *) echo "deploy.sh: unknown option $1" >&2; exit 64 ;;
    esac
done
[ "$(id -u)" -eq 0 ] || { echo "deploy.sh: run as root" >&2; exit 1; }
for b in builtpanel-control-api bpctl; do [ -x "$BIN_DIR/$b" ] || { echo "deploy.sh: $BIN_DIR/$b is missing" >&2; exit 1; }; done
KEEP="/var/lib/builtpanel/previous-release"
install -d -m 0700 "$KEEP"
for b in builtpanel-control-api bpctl; do [ -f /usr/local/bin/$b ] && cp -p /usr/local/bin/$b "$KEEP/$b"; done
ready() { for _ in $(seq 1 60); do curl -fsS http://127.0.0.1:8080/api/v1/public/ready > /dev/null 2>&1 && return 0; sleep 1; done; return 1; }

echo "==> installing the new binaries"
for b in builtpanel-control-api bpctl; do install -m 0755 -o root -g root "$BIN_DIR/$b" "/usr/local/bin/$b.new" && mv -f "/usr/local/bin/$b.new" "/usr/local/bin/$b"; done
# The web interface, when the server has one (it was installed with one) and the release brings it.
WEB_ROOT=/usr/share/builtpanel/web
if [ -n "$WEB_DIR" ] && [ -d "$WEB_ROOT" ]; then
    echo "==> installing the web interface"
    for app in client admin; do
        [ -f "$WEB_DIR/web-$app" ] || continue
        rm -rf "$WEB_ROOT/$app.new" "$WEB_ROOT/$app.old"; mkdir "$WEB_ROOT/$app.new"
        tar -xzf "$WEB_DIR/web-$app" -C "$WEB_ROOT/$app.new" --no-same-owner --no-same-permissions
        [ -f "$WEB_ROOT/$app.new/index.html" ] || { echo "deploy.sh: the $app web interface has no index.html" >&2; rm -rf "$WEB_ROOT/$app.new"; continue; }
        chown -R root:root "$WEB_ROOT/$app.new"; chmod -R u=rwX,go=rX "$WEB_ROOT/$app.new"
        [ -d "$WEB_ROOT/$app" ] && mv "$WEB_ROOT/$app" "$WEB_ROOT/$app.old"
        mv "$WEB_ROOT/$app.new" "$WEB_ROOT/$app"
    done
fi
echo "==> migrating the database"
runuser -u builtpanel -- env BUILTPANEL_CONFIG=/etc/builtpanel/control.toml /usr/local/bin/bpctl migrate
echo "==> restarting the control plane"
systemctl restart builtpanel-control-api
if ready; then
    rm -rf "$WEB_ROOT/client.old" "$WEB_ROOT/admin.old"
    echo "==> the new version is up"
    exit 0
fi
echo "deploy.sh: the new version did not become ready; putting the old one back" >&2
for app in client admin; do
    [ -d "$WEB_ROOT/$app.old" ] && { rm -rf "$WEB_ROOT/$app"; mv "$WEB_ROOT/$app.old" "$WEB_ROOT/$app"; }
done
for b in builtpanel-control-api bpctl; do [ -f "$KEEP/$b" ] && install -m 0755 "$KEEP/$b" "/usr/local/bin/$b"; done
systemctl restart builtpanel-control-api
ready && echo "deploy.sh: the old version is running again" >&2 || echo "deploy.sh: the old version does not answer either: journalctl -u builtpanel-control-api" >&2
exit 1
