# Sends a release to the server named in DEPLOY_HOST and has it installed there (T19.11). The
# release workflows call this; DEPLOY_SSH_KEY and DEPLOY_KNOWN_HOSTS hold the key and the host's
# key line (never ask the first time: a changed host key must fail the deploy).
#
# The archive and the signed release manifest are copied to the deploy user's incoming folder,
# and the only command run is `sudo builtpanel-deploy NAME` (deploy/release/server-deploy.sh,
# put there by setup-deploy-user.sh), which installs nothing the release key did not sign.
#
#   DEPLOY_HOST=builtpanel-deploy@panel DEPLOY_SSH_KEY=... DEPLOY_KNOWN_HOSTS=... \
#     remote-deploy.sh builtpanel-1.0.0-linux-x86_64.tar.gz release-manifest.json
set -euo pipefail
tarball="${1:?usage: remote-deploy.sh TARBALL MANIFEST}"
manifest="${2:?usage: remote-deploy.sh TARBALL MANIFEST}"
: "${DEPLOY_HOST:?}" "${DEPLOY_SSH_KEY:?}" "${DEPLOY_KNOWN_HOSTS:?}"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
(umask 077; printf '%s\n' "$DEPLOY_SSH_KEY" > "$work/key"; printf '%s\n' "$DEPLOY_KNOWN_HOSTS" > "$work/known_hosts")
ssh_opts=(-i "$work/key" -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$work/known_hosts")
name="$(basename "$tarball" .tar.gz)"
scp "${ssh_opts[@]}" "$tarball" "$DEPLOY_HOST:incoming/$name.tar.gz"
scp "${ssh_opts[@]}" "$manifest" "$DEPLOY_HOST:incoming/release-manifest.json"
ssh "${ssh_opts[@]}" "$DEPLOY_HOST" "sudo -n /usr/local/sbin/builtpanel-deploy $name"
