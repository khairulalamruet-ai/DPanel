#!/usr/bin/env bash
# Prepares a server to receive releases from the release pipeline (run once, as root, on the
# panel server). It makes a deploy user that can do exactly one thing as root, `builtpanel-deploy`
# (server-deploy.sh), which only installs programs signed with a trusted release key.
#
#   scp -r deploy/release you@server:/tmp/builtpanel-release
#   ssh you@server 'sudo /tmp/builtpanel-release/setup-deploy-user.sh --pubkey-file /tmp/builtpanel-release/deploy-staging.pub'
#
# The public key is the pair of the private key kept in the GitHub environment's DEPLOY_SSH_KEY.
# Run it again to change the key: nothing else is touched.
set -euo pipefail
die() { echo "setup-deploy-user: $*" >&2; exit 1; }
[ "$(id -u)" -eq 0 ] || die "run as root"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PUBKEY=""
while [ $# -gt 0 ]; do
    case "$1" in
        --pubkey-file) PUBKEY="${2:?--pubkey-file needs a file}"; shift 2 ;;
        *) die "unknown option $1" ;;
    esac
done
[ -n "$PUBKEY" ] && [ -r "$PUBKEY" ] || die "give the deploy key's public file with --pubkey-file"
for f in server-deploy.sh deploy.sh release-public-keys.txt; do [ -r "$HERE/$f" ] || die "$f is missing next to this script"; done
KEYLINE="$(grep -E '^(ssh-ed25519|ssh-rsa|ecdsa-sha2-[a-z0-9]+) ' "$PUBKEY" | head -1)"
[ -n "$KEYLINE" ] || die "$PUBKEY holds no public key line"
USER_NAME=builtpanel-deploy
HOME_DIR=/var/lib/builtpanel-deploy

id "$USER_NAME" > /dev/null 2>&1 || useradd --system --create-home --home-dir "$HOME_DIR" --shell /bin/bash "$USER_NAME"
install -d -m 0750 -o "$USER_NAME" -g "$USER_NAME" "$HOME_DIR"
install -d -m 0700 -o "$USER_NAME" -g "$USER_NAME" "$HOME_DIR/incoming" "$HOME_DIR/.ssh"
# `restrict`: no terminal, no forwarding, no agent; the key can copy files and run the one command.
printf 'restrict %s\n' "$KEYLINE" > "$HOME_DIR/.ssh/authorized_keys"
chown "$USER_NAME:$USER_NAME" "$HOME_DIR/.ssh/authorized_keys"; chmod 0600 "$HOME_DIR/.ssh/authorized_keys"

install -d -m 0755 /usr/local/lib/builtpanel /etc/builtpanel
install -m 0755 -o root -g root "$HERE/server-deploy.sh" /usr/local/sbin/builtpanel-deploy
install -m 0755 -o root -g root "$HERE/deploy.sh" /usr/local/lib/builtpanel/deploy.sh
install -m 0644 -o root -g root "$HERE/release-public-keys.txt" /etc/builtpanel/release-public-keys.txt

rule="$(mktemp)"; trap 'rm -f "$rule"' EXIT
printf '%s ALL=(root) NOPASSWD: /usr/local/sbin/builtpanel-deploy builtpanel-*\n' "$USER_NAME" > "$rule"
visudo -cf "$rule" > /dev/null || die "the sudoers rule is not valid"
install -m 0440 -o root -g root "$rule" /etc/sudoers.d/builtpanel-deploy

echo "ready. The deploy user is $USER_NAME; its key may only copy to $HOME_DIR/incoming and run builtpanel-deploy."
echo "DEPLOY_HOST is: $USER_NAME@<this server's address>"
echo "This server's own key, for DEPLOY_KNOWN_HOSTS (put the address in front):"
cat /etc/ssh/ssh_host_ed25519_key.pub 2> /dev/null | awk '{print "<address> " $1 " " $2}' || true
