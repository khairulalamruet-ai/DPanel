#!/usr/bin/env bash
# The panel's own update, run as root by builtpanel-update.service when the control plane leaves a
# request (an administrator pressed "Update now"). Installed as /usr/local/sbin/builtpanel-self-update.
#
# The control plane never runs as root and cannot choose what this runs: all it can do is put a
# version number (X.Y.Z, nothing else is read) in the request file. This script then
#   1. fetches that release's archive and its signed manifest from the release site,
#   2. lets builtpanel-deploy (deploy/release/server-deploy.sh) check every program and web file
#      against the manifest with the trusted release keys, and install them with deploy.sh,
#      which migrates, restarts, waits for the panel and puts the old version back when the new
#      one does not come up,
#   3. leaves what happened in /var/lib/builtpanel-update (status.json, history.json, last.log),
#      which only root writes and the panel reads.
#
# /etc/builtpanel/update.conf (root's file, optional) may set REPO=owner/name or BASE_URL=https://...
set -uo pipefail

REQ_DIR=/var/lib/builtpanel/update-request
REQ="$REQ_DIR/request"
OUT="${BUILTPANEL_UPDATE_OUT:-/var/lib/builtpanel-update}"
DEPLOY="${BUILTPANEL_DEPLOY_COMMAND:-/usr/local/sbin/builtpanel-deploy}"
BPCTL="${BUILTPANEL_BPCTL:-/usr/local/bin/bpctl}"
REPO="khairulalamruet-ai/DPanel"; BASE_URL=""
# shellcheck disable=SC1091
[ ! -r /etc/builtpanel/update.conf ] || . /etc/builtpanel/update.conf
[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || REPO="khairulalamruet-ai/DPanel"
BASE_URL="${BASE_URL:-https://github.com/$REPO/releases/download}"
[[ "$BASE_URL" =~ ^https:// ]] || { echo "self-update: BASE_URL must start with https://" >&2; exit 1; }

install -d -m 0755 -o root -g root "$OUT"
exec 9> "$OUT/lock"
flock -n 9 || { echo "self-update: an update is already running" >&2; exit 0; }

# status STATE STEP VERSION FROM MESSAGE: written whole, then renamed into place.
status() {
    python3 - "$OUT/status.json" "$1" "$2" "$3" "$4" "$5" <<'PY'
import json, os, sys, time
path, state, step, version, old, message = sys.argv[1:7]
doc = {"state": state, "step": step, "version": version, "from_version": old, "message": message[:2000],
       "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
tmp = path + ".tmp"
with open(tmp, "w") as f:
    json.dump(doc, f)
os.chmod(tmp, 0o644)
os.replace(tmp, path)
PY
}
# history_add STATUS VERSION FROM DETAILS: the newest first, twenty kept.
history_add() {
    python3 - "$OUT/history.json" "$1" "$2" "$3" "$4" <<'PY'
import json, os, sys, time, uuid
path, status, version, old, details = sys.argv[1:6]
try:
    items = json.load(open(path))
except Exception:
    items = []
items.insert(0, {"id": uuid.uuid4().hex, "from_version": old, "to_version": version, "channel": "stable",
                 "status": status, "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                 "details": details[:1000]})
tmp = path + ".tmp"
with open(tmp, "w") as f:
    json.dump(items[:20], f)
os.chmod(tmp, 0o644)
os.replace(tmp, path)
PY
}

OLD="$("$BPCTL" --version 2> /dev/null | awk '{print $NF}')"; OLD="${OLD:-unknown}"

# --- the request: one version number --------------------------------------------------------
if [ ! -f "$REQ" ] || [ -L "$REQ" ]; then exit 0; fi
VER="$(head -c 64 "$REQ" | tr -d ' \t\r\n')"
rm -f "$REQ"
if ! [[ "$VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    status failed request "" "$OLD" "The request is not a version number."
    exit 1
fi
fail() { status failed "$1" "$VER" "$OLD" "$2"; history_add failed "$VER" "$OLD" "$2"; exit 1; }
[ "$(uname -m)" = x86_64 ] || fail request "Automatic updates are built for x86_64 servers; this one is $(uname -m)."
if [ "$OLD" != unknown ]; then
    newest="$(printf '%s\n%s\n' "$OLD" "$VER" | sort -V | tail -1)"
    if [ "$OLD" = "$VER" ] || [ "$newest" != "$VER" ]; then fail request "Version $VER is not newer than the installed $OLD."; fi
fi
NAME="builtpanel-$VER-linux-x86_64"

WORK="$(mktemp -d /var/tmp/builtpanel-update.XXXXXX)"; chmod 0700 "$WORK"
trap 'rm -rf "$WORK"' EXIT
mkdir "$WORK/incoming"

# --- fetch ----------------------------------------------------------------------------------
status running downloading "$VER" "$OLD" "Downloading release $VER."
get() { curl -fsSL --proto '=https' --tlsv1.2 --retry 3 --max-time 900 -o "$2" "$BASE_URL/v$VER/$1"; }
get "$NAME.tar.gz" "$WORK/incoming/$NAME.tar.gz" || fail downloading "The release archive could not be downloaded ($BASE_URL/v$VER/$NAME.tar.gz)."
get release-manifest.json "$WORK/incoming/release-manifest.json" || fail downloading "The signed release manifest could not be downloaded; releases without it are not installed."

# --- check and install ------------------------------------------------------------------------
status running verifying "$VER" "$OLD" "Checking the release signature and installing."
if BUILTPANEL_DEPLOY_STATE="$WORK" "$DEPLOY" "$NAME" > "$OUT/last.log" 2>&1; then
    chmod 0644 "$OUT/last.log"
    status completed completed "$VER" "$OLD" "Updated from $OLD to $VER."
    history_add success "$VER" "$OLD" "The release was verified against its signed manifest and installed."
    exit 0
fi
chmod 0644 "$OUT/last.log"
reason="$(tail -n 5 "$OUT/last.log" | tr '\n' ' ')"
if grep -q "the old version is running again" "$OUT/last.log"; then
    status rolled_back rolled_back "$VER" "$OLD" "The new version did not start; $OLD is running again. $reason"
    history_add rolled_back "$VER" "$OLD" "$reason"
else
    status failed failed "$VER" "$OLD" "$reason"
    history_add failed "$VER" "$OLD" "$reason"
fi
exit 1
