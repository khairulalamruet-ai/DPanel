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
#   --no-upgrade         do not update the operating system first (by default every package, the
#                        kernel included, is brought up to date: apt on Ubuntu/Debian, dnf on
#                        AlmaLinux/Rocky/RHEL/CentOS Stream; nothing is rebooted for you)
#   --no-resume          on AlmaLinux/Rocky/RHEL, when the update needs a reboot first: do not set up
#                        the automatic continuation after it (run this command again yourself)
#   --check              download and verify, then only look at what would be done
#
# The first administrator holds the built-in `super_admin` role (every permission, like WHM's
# root); the generated password is printed once at the end. Anything else is passed to install.sh.
set -euo pipefail

REPO="khairulalamruet-ai/DPanel"; BASE_URL=""; HEADER=(); VERSION=""; TARBALL=""; HOSTNAME_FQDN=""; ADMIN_EMAIL=""; UPGRADE=1; CHECK_ONLY=0; RESUME=1
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
        --no-upgrade) UPGRADE=0; shift ;;
        --no-resume) RESUME=0; shift ;;
        --check) CHECK_ONLY=1; PASS+=("$1"); shift ;;
        -h | --help) sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
UI_TOTAL=$((6 + 2 * (UPGRADE - CHECK_ONLY * UPGRADE)))
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

# --- the operating system ---------------------------------------------------------------------
# Brought up to date before anything is installed: the security fixes and the kernel, by the way of the
# distribution (apt on Ubuntu and Debian, dnf on AlmaLinux, Rocky, RHEL and CentOS Stream). Only on a
# server the installer accepts (a system it refuses is not touched), and never with --check.
# Nothing is restarted or rebooted here; a new kernel runs after the next reboot.
REBOOT_NEEDED=0
os_family() {
    local ids
    # In a subshell: os-release sets VERSION and other names this script uses.
    ids="$( (. /etc/os-release 2> /dev/null; echo "${ID:-} ${ID_LIKE:-}") )"
    case " $ids " in
        *" ubuntu "* | *" debian "*) echo deb ;;
        *" rhel "* | *" centos "* | *" fedora "* | *" almalinux "* | *" rocky "*) echo rhel ;;
        *) echo "" ;;
    esac
}
# After the reboot a one-shot service finishes the installation from the release that was just
# downloaded and checked (kept in /var/lib/builtpanel-resume, root only), then removes itself.
# The installer's final summary goes to /var/log/builtpanel-resume.log (mode 0600).
resume_after_reboot() {
    command -v systemctl > /dev/null || return 1
    local dir=/var/lib/builtpanel-resume a
    rm -rf "$dir"; mkdir -p "$dir"; chmod 700 "$dir"
    cp -a "$WORK/release" "$dir/release" || return 1
    {
        printf '#!/usr/bin/env bash\n'
        printf 'set -uo pipefail\n'
        # Whatever happens, it runs once: a failed installation is not retried in a loop.
        printf 'systemctl disable builtpanel-resume.service > /dev/null 2>&1\n'
        printf 'rm -f /etc/systemd/system/builtpanel-resume.service\n'
        printf 'BPINSTALL_REBOOT_NEEDED=0 BPINSTALL_BANNER=0 TERM=dumb %q --no-upgrade --bin-dir %q --hostname %q --admin-email %q' \
            "$dir/release/install.sh" "$dir/release/bin" "$HOSTNAME_FQDN" "$ADMIN_EMAIL"
        for a in ${PASS[@]+"${PASS[@]}"}; do printf ' %q' "$a"; done
        printf '\nstatus=$?\n'
        printf 'echo "builtpanel-resume: the installer ended with $status"\n'
        printf 'rm -rf %q\n' "$dir"
        printf 'exit $status\n'
    } > "$dir/run.sh"
    chmod 700 "$dir/run.sh"
    ( umask 077; : > /var/log/builtpanel-resume.log )
    cat > /etc/systemd/system/builtpanel-resume.service << UNIT
[Unit]
Description=Finish the Built Panel installation after the reboot
After=network-online.target
Wants=network-online.target
ConditionPathExists=$dir/run.sh

[Service]
Type=oneshot
ExecStart=/bin/bash $dir/run.sh
StandardOutput=append:/var/log/builtpanel-resume.log
StandardError=append:/var/log/builtpanel-resume.log
TimeoutStartSec=3600

[Install]
WantedBy=multi-user.target
UNIT
    systemctl daemon-reload && systemctl enable builtpanel-resume.service > /dev/null 2>&1
}
os_update() {
    local log=/var/log/builtpanel-os-update.log family n newest
    family="$(os_family)"
    : > "$log"
    case "$family" in
        deb)
            export DEBIAN_FRONTEND=noninteractive NEEDRESTART_MODE=l NEEDRESTART_SUSPEND=1
            # A new machine often runs its own first update at boot: wait for the package lock (up to 5
            # minutes). Edited configuration files are kept. full-upgrade, because a new kernel is a new package.
            local opts=(-o DPkg::Lock::Timeout=300 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold)
            # DPkg::Lock::Timeout covers only the dpkg lock; the lists lock of `apt-get update` fails at once while
            # the machine's own first update holds it. So each step is tried again while the log says "lock".
            apt_step() {
                local t
                for t in $(seq 1 40); do
                    : > "$log.step"
                    if "$@" >> "$log.step" 2>&1; then cat "$log.step" >> "$log"; return 0; fi
                    cat "$log.step" >> "$log"
                    grep -qi 'could not get lock\|unable to lock\|is held by process' "$log.step" || return 1
                    sleep 15
                done
                return 1
            }
            apt_step apt-get "${opts[@]}" update -qq && apt_step apt-get "${opts[@]}" full-upgrade -y -qq || return 1
            apt-get "${opts[@]}" autoremove -y -qq >> "$log" 2>&1 || true
            [ ! -e /run/reboot-required ] || REBOOT_NEEDED=1
            ;;
        rhel)
            for n in 1 2 3; do
                dnf upgrade -y -q --refresh --setopt=retries=5 --setopt=timeout=60 >> "$log" 2>&1 && break
                [ "$n" -lt 3 ] || return 1
                sleep 5
            done
            newest="$(rpm -q kernel-core --qf '%{VERSION}-%{RELEASE}.%{ARCH}\n' --last 2> /dev/null | head -1)" || newest=""
            [ -z "$newest" ] || [ "$newest" = "$(uname -r)" ] || REBOOT_NEEDED=1
            ;;
        *)
            echo "get.sh: warning: this Linux is not Debian- or Red Hat-based: its system update is skipped" >&2
            ;;
    esac
}
if [ "$UPGRADE" -eq 1 ] && [ "$CHECK_ONLY" -eq 0 ]; then
    say "checking this server"
    # The installer's own look at the server (system, memory, disk, programs) decides: nothing below runs on a
    # server it would refuse.
    if ! "$WORK/release/install.sh" --check --bin-dir "$WORK/release/bin" --hostname "$HOSTNAME_FQDN" --admin-email "$ADMIN_EMAIL" ${PASS[@]+"${PASS[@]}"} > "$WORK/check.log" 2>&1; then
        _ui_end_step
        tail -n 8 "$WORK/check.log" >&2
        die "this server cannot run Built Panel (see above): nothing was changed"
    fi
    say "updating the operating system"
    os_update || die "the system update failed (the end of /var/log/builtpanel-os-update.log: $(tail -n 3 /var/log/builtpanel-os-update.log | tr '\n' ' ')); fix it, or run this again with --no-upgrade"
    if [ "$REBOOT_NEEDED" = 1 ] && [ "$(os_family)" = rhel ] && ! printf '%s\n' ${PASS[@]+"${PASS[@]}"} | grep -qx -- --no-agent; then
        # Docker's network modules must match the running kernel: the new one has to run first.
        _ui_end_step
        if [ "$RESUME" = 1 ] && resume_after_reboot; then
            echo "get.sh: NOT an error - the system update installed a newer kernel, and it has to be running first." >&2
            echo "        Run  reboot  : the installation goes on by itself after the restart (about 5 minutes)." >&2
            echo "        Follow it with  journalctl -u builtpanel-resume -f  ; when it is done, its summary (with the" >&2
            echo "        panel's address and login) is in /var/log/builtpanel-resume.log (root only): read it, then delete it." >&2
        else
            echo "get.sh: NOT an error - the system update installed a newer kernel, and it has to be running first." >&2
            echo "        Run  reboot  and then this same command again: the update is done, the second run installs the panel." >&2
        fi
        exit 0
    fi
fi

say "starting the installer"
_ui_end_step
# The installer goes on with the same look, without drawing the logo again. The system update is done here
# (or was asked not to be), so it does not repeat it; it only needs to know whether a reboot is due.
BPINSTALL_REBOOT_NEEDED="$REBOOT_NEEDED" BPINSTALL_BANNER=0 "$WORK/release/install.sh" --no-upgrade --bin-dir "$WORK/release/bin" --hostname "$HOSTNAME_FQDN" --admin-email "$ADMIN_EMAIL" ${PASS[@]+"${PASS[@]}"}
