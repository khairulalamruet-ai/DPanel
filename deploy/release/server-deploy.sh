#!/usr/bin/env bash
# The one thing the deploy user may run as root (installed by setup-deploy-user.sh as
# /usr/local/sbin/builtpanel-deploy; sudoers lets that user run exactly this and nothing else):
#
#   sudo builtpanel-deploy builtpanel-1.2.3-linux-x86_64
#
# The release pipeline puts two files in the deploy user's incoming folder: the archive and
# release-manifest.json (signed with a release key). Nothing from the archive is run: its
# programs are checked against the signed manifest with the release keys this server trusts
# (bpctl release verify), and only then does the deploy script that is installed here, owned
# by root, put them in place. A stolen deploy key therefore cannot make this server run
# anything that the release key did not sign.
set -euo pipefail
die() { echo "builtpanel-deploy: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || die "run as root (through sudo)"
NAME="${1:-}"
[[ "$NAME" =~ ^builtpanel-[0-9]+\.[0-9]+\.[0-9]+-linux-x86_64$ ]] || die "not a release name: $NAME"
STATE="${BUILTPANEL_DEPLOY_STATE:-/var/lib/builtpanel-deploy}"
KEYS="${BUILTPANEL_RELEASE_KEYS:-/etc/builtpanel/release-public-keys.txt}"
DEPLOY="${BUILTPANEL_DEPLOY_SCRIPT:-/usr/local/lib/builtpanel/deploy.sh}"
BPCTL="${BUILTPANEL_BPCTL:-/usr/local/bin/bpctl}"
[ -r "$KEYS" ] || die "no trusted release keys at $KEYS"
[ -x "$BPCTL" ] || die "bpctl is not installed at $BPCTL (install the panel first)"
[ -x "$DEPLOY" ] || die "no deploy script at $DEPLOY"

work="$(mktemp -d "$STATE/work.XXXXXX")"; chmod 0700 "$work"
trap 'rm -rf "$work"' EXIT
# Copies the deploy user cannot change any more, before anything is looked at.
for f in "$NAME.tar.gz" release-manifest.json; do
    [ -f "$STATE/incoming/$f" ] && [ ! -L "$STATE/incoming/$f" ] || die "$f is not in the incoming folder"
    install -m 0600 -o root -g root "$STATE/incoming/$f" "$work/$f"
done
rm -f "$STATE/incoming/$NAME.tar.gz" "$STATE/incoming/release-manifest.json"

# No member may leave the folder it is unpacked into. (The list is read whole and searched without a
# pipe: `tar | grep -q` can end with tar killed by SIGPIPE, which `pipefail` then reports as "not found".)
LIST="$(tar -tzf "$work/$NAME.tar.gz")" || die "the archive cannot be read"
if grep -Eq '^/|(^|/)\.\.(/|$)' <<< "$LIST"; then die "the archive has unsafe paths"; fi
mkdir "$work/x"
tar -xzf "$work/$NAME.tar.gz" -C "$work/x" --no-same-owner --no-same-permissions "$NAME/bin"
BIN="$work/x/$NAME/bin"
[ -d "$BIN" ] || die "the archive has no bin folder"
# The web interface comes with a release too (two archives); a release without it is fine.
WEB=""
if grep -q "^$NAME/web/" <<< "$LIST"; then
    tar -xzf "$work/$NAME.tar.gz" -C "$work/x" --no-same-owner --no-same-permissions "$NAME/web"
    WEB="$work/x/$NAME/web"
fi
for dir in "$BIN" $WEB; do
    [ -z "$(find "$dir" -mindepth 1 \( -type l -o -type d \) -print -quit)" ] || die "$dir holds more than plain files"
done

# What is installed is what the signed manifest says (every file of it, byte for byte): the
# programs and the web archives are put side by side so that one check covers them.
STAGE="$work/stage"; mkdir "$STAGE"
cp "$BIN"/* "$STAGE"/
[ -z "$WEB" ] || cp "$WEB"/* "$STAGE"/
"$BPCTL" release verify --manifest "$work/release-manifest.json" --keys "$KEYS" --dir "$STAGE"
# ...and what this server installs is among them (a manifest of other files will not do).
python3 - "$work/release-manifest.json" "${WEB:+web}" <<'PY'
import base64, json, sys
doc = json.load(open(sys.argv[1]))
payload = doc["payload"] + "=" * (-len(doc["payload"]) % 4)
names = {c["name"] for c in json.loads(base64.urlsafe_b64decode(payload))["components"]}
needed = {"builtpanel-control-api", "bpctl"}
if len(sys.argv) > 2 and sys.argv[2]:
    needed |= {"web-client", "web-admin"}
missing = needed - names
if missing:
    sys.exit("builtpanel-deploy: the signed manifest does not list " + ", ".join(sorted(missing)))
PY
# Every program the deploy script installs is one of the checked files. (Not `exec`: the
# working folder is removed when this ends.)
"$DEPLOY" --bin-dir "$BIN" ${WEB:+--web-dir "$WEB"}
