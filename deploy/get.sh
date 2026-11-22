#!/usr/bin/env bash
# One-command installer: downloads a Built Panel release, checks it and runs install.sh from it.
#
#   curl -fsSL https://github.com/khairulalamruet-ai/DPanel/releases/latest/download/get.sh | sudo bash
#
# That is all that is needed on a new Ubuntu 22.04+/Debian 12+ or AlmaLinux/Rocky/RHEL/CentOS Stream
# 9+ server: the panel is then served under <ip>.sslip.io (no DNS to set up) and the first
# administrator is `owner`. Options go after `-s --`:
#
#   curl -fsSL .../get.sh | sudo bash -s -- --hostname panel.example.com --admin-email me@example.com
#
#   --hostname NAME      the panel's DNS name (default: <public-ip-with-dashes>.sslip.io)
#   --admin-email MAIL   the first administrator's email (default: admin@<hostname>)
#   --admin-username U   the first administrator's login (default: owner)
#   --version X.Y.Z      a release other than the latest
#   --repo OWNER/NAME    where releases are published (default: khairulalamruet-ai/DPanel)
#   --url BASE           download the archive and its .sha256 from BASE (any https address, e.g. a
#                        private server or bucket) instead of GitHub
#   --header 'H: V'      an HTTP header for the download (e.g. 'Authorization: Bearer TOKEN')
#   --tarball FILE       install from a release archive already on this machine (no download)
#   --no-agent           the control plane only; nodes are added from other servers
#   --no-upgrade         do not update the operating system (packages and kernel) first
#   --check              download and verify, then only look at what would be done
#
# The first administrator holds the built-in `super_admin` role (every permission, like WHM's
# root); the generated password is printed once at the end. Anything else is passed to install.sh.
set -euo pipefail

REPO="khairulalamruet-ai/DPanel"; BASE_URL=""; HEADER=(); VERSION=""; TARBALL=""; HOSTNAME_FQDN=""; ADMIN_EMAIL=""
PASS=()

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) REPO="${2:?--repo needs OWNER/NAME}"; shift 2 ;;
        --version) VERSION="${2:?--version needs a number}"; VERSION="${VERSION#v}"; shift 2 ;;
        --url) BASE_URL="${2:?--url needs an address}"; BASE_URL="${BASE_URL%/}"; shift 2 ;;
        --header) HEADER=(-H "${2:?--header needs 'Name: value'}"); shift 2 ;;
        --tarball) TARBALL="${2:?--tarball needs a file}"; shift 2 ;;
        --hostname) HOSTNAME_FQDN="${2:?--hostname needs a name}"; shift 2 ;;
        --admin-email) ADMIN_EMAIL="${2:?--admin-email needs an address}"; shift 2 ;;
        -h | --help) sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) PASS+=("$1"); shift ;;
    esac
done

# The installer's look (deploy/ui.sh). The published get.sh carries a copy of it right below
# the marker (the release puts it there); run from a checkout, it reads the file next to it;
# otherwise the steps are plain lines.
# @UI@
if ! declare -F ui_init > /dev/null; then
    if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "$(dirname "${BASH_SOURCE[0]}")/ui.sh" ]; then
        # shellcheck source=deploy/ui.sh
        . "$(dirname "${BASH_SOURCE[0]}")/ui.sh"
    else
        ui_init() { exec 3>&1 4>&2; }
        ui_banner() { :; }
        ui_step() { echo "==> $1"; }
        ui_fail() { echo "get.sh: $1" >&2; }
        ui_cleanup() { :; }
        ui_is_main() { :; }
        _ui_end_step() { :; }
    fi
fi
UI_TOTAL=6
ui_init
ui_banner "the modern hosting control panel  ·  installer"

die() { ui_fail "$*"; exit 1; }
say() { ui_step "$*"; }

[ "$(uname -s)" = Linux ] || die "Linux only"
[ "$(id -u)" -eq 0 ] || die "run as root: curl -fsSL <url> | sudo bash"
[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "--repo must look like OWNER/NAME"
case "$(uname -m)" in x86_64) ARCH=x86_64 ;; aarch64 | arm64) ARCH=aarch64 ;; *) die "x86_64 or aarch64 only" ;; esac

if ! command -v curl > /dev/null || ! command -v tar > /dev/null; then
    if command -v apt-get > /dev/null; then
        DEBIAN_FRONTEND=noninteractive apt-get update -qq > /dev/null
        DEBIAN_FRONTEND=noninteractive apt-get install -y -qq curl tar ca-certificates > /dev/null
    elif command -v dnf > /dev/null; then
        # curl-minimal (the default) is enough: only what is missing is installed.
        command -v curl > /dev/null || dnf install -y -q curl > /dev/null
        command -v tar > /dev/null || dnf install -y -q tar > /dev/null
    else
        die "curl and tar are needed"
    fi
fi

WORK="$(mktemp -d /var/tmp/builtpanel-get.XXXXXX)"
# Only the script itself may run this: the spinner is a copy of it, and would delete the folder.
trap 'if ui_is_main; then ui_cleanup; rm -rf "$WORK"; fi' EXIT

# --- the release ----------------------------------------------------------------------------
if [ -n "$TARBALL" ]; then
    [ -f "$TARBALL" ] || die "$TARBALL is not a file"
    cp "$TARBALL" "$WORK/release.tar.gz"
    [ ! -f "$TARBALL.sha256" ] || { cp "$TARBALL.sha256" "$WORK/release.sha256"; }
elif [ -n "$BASE_URL" ]; then
    [[ "$BASE_URL" =~ ^https:// ]] || die "--url must start with https://"
    # The archive is builtpanel-<version>-linux-<arch>.tar.gz; without --version the newest one named
    # in BASE/latest (a one-line text file holding the version) is taken.
    if [ -z "$VERSION" ]; then
        VERSION=$(curl -fsSL ${HEADER[@]+"${HEADER[@]}"} "$BASE_URL/latest" | tr -d ' \r\n') || die "cannot read $BASE_URL/latest (or give --version)"
    fi
    [[ "$VERSION" =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]] || die "the version does not look like a version number"
    name="builtpanel-${VERSION}-linux-${ARCH}.tar.gz"
    say "downloading Built Panel $VERSION ($ARCH) from $BASE_URL"
    curl -fsSL --retry 3 ${HEADER[@]+"${HEADER[@]}"} -o "$WORK/release.tar.gz" "$BASE_URL/$name" || die "cannot download $BASE_URL/$name"
    curl -fsSL --retry 3 ${HEADER[@]+"${HEADER[@]}"} -o "$WORK/release.sha256" "$BASE_URL/$name.sha256" || die "cannot download the checksum"
else
    if [ -z "$VERSION" ]; then
        say "looking for the latest release of $REPO"
        # The page /releases/latest redirects to /releases/tag/<tag>. (Not api.github.com: it
        # allows 60 requests an hour per address and then answers 403, e.g. on shared hosts.)
        tag=$(curl -fsSIL -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed -n 's#.*/releases/tag/\([^/?]*\)$#\1#p')
        [ -n "$tag" ] || die "no release found for $REPO (give one with --version)"
        VERSION="${tag#v}"
    fi
    [[ "$VERSION" =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]] || die "--version does not look like a version number"
    name="builtpanel-${VERSION}-linux-${ARCH}.tar.gz"
    base="https://github.com/$REPO/releases/download/v${VERSION}"
    say "downloading Built Panel $VERSION ($ARCH)"
    curl -fsSL --retry 3 -o "$WORK/release.tar.gz" "$base/$name" || die "cannot download $base/$name"
    curl -fsSL --retry 3 -o "$WORK/release.sha256" "$base/$name.sha256" || die "cannot download the checksum $base/$name.sha256"
fi
# The checksum is published next to the archive (the signed manifest of the release covers it
# again, and `bpctl release verify` checks the installed programs against it later).
if [ -f "$WORK/release.sha256" ]; then
    say "checking the download"
    want=$(awk '{print $1; exit}' "$WORK/release.sha256")
    have=$(sha256sum "$WORK/release.tar.gz" | awk '{print $1}')
    [ "$want" = "$have" ] || die "the archive does not match its checksum (expected $want, got $have): nothing was installed"
elif [ -z "$TARBALL" ]; then
    die "no checksum for the archive"
else
    echo "get.sh: warning: no $TARBALL.sha256 next to the archive, so it is not checked" >&2
fi

say "unpacking"
mkdir "$WORK/release"
tar -xzf "$WORK/release.tar.gz" -C "$WORK/release" --strip-components=1 --no-same-owner
[ -x "$WORK/release/install.sh" ] && [ -d "$WORK/release/bin" ] || die "the archive is not a Built Panel release (no install.sh and bin/)"

# --- defaults that make it one command -------------------------------------------------------
if [ -z "$HOSTNAME_FQDN" ]; then
    # sslip.io answers any name of the form a-b-c-d.sslip.io with that address, so the panel (and
    # admin.<name>) work and get a certificate without any DNS setting.
    ip=$(curl -4 -fsS --max-time 10 https://api.ipify.org 2> /dev/null || true)
    [[ "$ip" =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]] || die "cannot find this server's public address; give a name with --hostname"
    HOSTNAME_FQDN="${ip//./-}.sslip.io"
    say "no --hostname given: using $HOSTNAME_FQDN"
fi
[ -n "$ADMIN_EMAIL" ] || ADMIN_EMAIL="admin@$HOSTNAME_FQDN"

say "starting the installer"
_ui_end_step
# The installer goes on with the same look, without drawing the logo again.
BPINSTALL_BANNER=0 "$WORK/release/install.sh" --bin-dir "$WORK/release/bin" --hostname "$HOSTNAME_FQDN" --admin-email "$ADMIN_EMAIL" ${PASS[@]+"${PASS[@]}"}
