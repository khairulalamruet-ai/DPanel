#!/usr/bin/env bash
# Installs the Built Panel control plane, and by default a local node agent, on ONE new server
# (Ubuntu 22.04/24.04/26.04, Debian 12/13, or AlmaLinux/Rocky/RHEL 9 and 10; run as root). T19.01.
#
#   sudo ./install.sh --bin-dir ./bin --hostname panel.example.com --admin-email you@example.com
#
#   --bin-dir DIR        the release's binaries: builtpanel-control-api, bpctl, builtpanel-root-auth, and (for the local
#                        node) builtpanel-agent and builtpanel-priv-helper
#   --hostname NAME      the DNS name the panel is served under (it must already point here;
#                        Caddy gets its certificate from it)
#   --admin-email MAIL   the first administrator's email
#   --admin-username U   the first administrator's login (default: owner; "admin" and other system names are reserved)
#   --web-dir DIR        the built web interface: the two archives of a release (DIR/web-client and
#                        DIR/web-admin) or the two built folders (DIR/client and DIR/admin). Default:
#                        the web folder of the release this script is in, when there is one. Without
#                        a web interface only the API is served.
#   --admin-hostname N   the name the administrators' app is served under (default: admin.<hostname>;
#                        it must point here too; on sslip.io names it does on its own)
#   --no-agent           install the control plane only (nodes are added from other servers)
#   --no-root-login      do not let `root` log in to the panel with this server's root password
#                        (on by default, like WHM; the helper builtpanel-root-auth checks it)
#   --no-upgrade         do not update the operating system first (by default every installed package,
#                        the kernel included, is brought up to date before anything is installed)
#   --check              only look: what is there, what would be done; changes nothing
#
# Safe to run again: every step looks at what is already there, secrets and the CA are never
# replaced, an administrator is created only when there is none. When a step fails it says
# which and why; running again continues from there.
#
# What it installs: PostgreSQL 18 (PGDG), Redis, Caddy, a firewall (ufw, or firewalld on the Red Hat
# family: 22, 80, 443 and the agent port 8444), the control plane as the unprivileged user `builtpanel` under its hardened unit, the CA
# of the node channel, and with the local node: rootless Docker, the agent and the root helper.
set -euo pipefail

BIN_DIR=""; WEB_DIR=""; ADMIN_HOSTNAME=""; HOSTNAME_FQDN=""; ADMIN_EMAIL=""; ADMIN_USER=owner; WITH_AGENT=1; ROOT_LOGIN=1; UPGRADE=1; CHECK=0
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETC=/etc/builtpanel; SECRETS=$ETC/secrets; STATE=/var/lib/builtpanel; AGENT_PORT=8444

while [ $# -gt 0 ]; do
    case "$1" in
        --bin-dir) BIN_DIR="${2:?--bin-dir needs a folder}"; shift 2 ;;
        --hostname) HOSTNAME_FQDN="${2:?--hostname needs a name}"; shift 2 ;;
        --web-dir) WEB_DIR="${2:?--web-dir needs a folder}"; shift 2 ;;
        --admin-hostname) ADMIN_HOSTNAME="${2:?--admin-hostname needs a name}"; shift 2 ;;
        --admin-email) ADMIN_EMAIL="${2:?--admin-email needs an address}"; shift 2 ;;
        --admin-username) ADMIN_USER="${2:?--admin-username needs a name}"; shift 2 ;;
        --no-agent) WITH_AGENT=0; shift ;;
        --no-root-login) ROOT_LOGIN=0; shift ;;
        --no-upgrade) UPGRADE=0; shift ;;
        --check) CHECK=1; shift ;;
        -h | --help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "install.sh: unknown option: $1 (see --help)" >&2; exit 64 ;;
    esac
done

# The look: the logo, a live line per step with an overall progress bar (deploy/ui.sh), or plain
# "==> step" lines when that file is missing or the output is not a terminal.
if [ -f "$HERE/ui.sh" ]; then
    # shellcheck source=deploy/ui.sh
    . "$HERE/ui.sh"
else
    ui_init() { exec 3>&1 4>&2; }
    ui_banner() { :; }
    ui_log_to() { :; }
    ui_step() { echo "==> $1"; }
    ui_warn() { echo "warning: $1"; }
    ui_fail() { echo "install.sh: $1" >&2; }
    ui_done() { :; }
    ui_box() { cat >&3; }
    ui_cleanup() { :; }
    ui_is_main() { :; }
fi
UI_TOTAL=$(grep -c '^[[:space:]]*say "' "${BASH_SOURCE[0]}")
ui_init
[ "${BPINSTALL_BANNER:-1}" = 0 ] || ui_banner "the modern hosting control panel  ·  installer"

step=""
say() { step="$*"; ui_step "$*"; }
die() { ui_fail "$*"; exit 1; }
trap 'rc=$?; ui_is_main || exit $rc; ui_cleanup; if [ $rc -ne 0 ] && [ -n "$step" ]; then { echo; echo "install.sh: stopped during \"$step\" (the end of /var/log/builtpanel-install.log):"; tail -n 15 /var/log/builtpanel-install.log 2> /dev/null | sed "s/^/  | /"; echo "fix the cause and run this again"; } >&4; fi' EXIT

# --- what this machine is ------------------------------------------------------------------
# sudo on the Red Hat family does not have /usr/local/bin in its PATH, where the programs go.
export PATH="/usr/local/bin:/usr/local/sbin:/usr/sbin:/sbin:$PATH"
FAMILY=deb   # deb: Ubuntu and Debian (apt, ufw); rhel: AlmaLinux, Rocky, RHEL (dnf, firewalld, SELinux)
PG_BIN=/usr/lib/postgresql/18/bin; PG_SERVICE=postgresql; REDIS_SERVICE=redis-server; REDIS_CONF=/etc/redis/redis.conf
[ "$(uname -s)" = Linux ] || die "Linux only"
. /etc/os-release
case "${ID:-}:${VERSION_ID:-}" in
    ubuntu:22.04 | ubuntu:24.04 | ubuntu:26.04 | debian:12 | debian:13) CODENAME="$VERSION_CODENAME" ;;
    ubuntu:20.04 | ubuntu:18.04 | ubuntu:16.04)
        die "Ubuntu ${VERSION_ID} is not supported: it starts with the old cgroup v1 (a hosting account's memory and CPU limits do not work with rootless Docker there), has kernel 5.4 and no PostgreSQL 18 packages. Use Ubuntu 22.04 or newer." ;;
    centos:7* | rhel:7* | ol:7* | rocky:7*)
        die "${PRETTY_NAME:-this system} is not supported: kernel 3.10 cannot run rootless containers, there is no PostgreSQL 18 for it, and it is out of support. Use Ubuntu 22.04 or newer, Debian 12 or newer." ;;
    almalinux:9 | almalinux:9.* | almalinux:10 | almalinux:10.* | rocky:9 | rocky:9.* | rocky:10 | rocky:10.* | rhel:9 | rhel:9.* | rhel:10 | rhel:10.* | centos:9 | centos:10 | ol:9 | ol:9.* | ol:10 | ol:10.*)
        FAMILY=rhel; CODENAME="" ;;
    almalinux:* | rocky:* | rhel:* | centos:* | ol:*)
        die "${PRETTY_NAME:-this system} is not supported: version 8 starts with the old cgroup v1 and has kernel 4.18 (rootless containers with memory and CPU limits do not work). Use AlmaLinux, Rocky Linux or RHEL 9 or 10 (or Ubuntu 22.04 or newer, Debian 12 or newer)." ;;
    *) die "this installer supports Ubuntu 22.04, 24.04 and 26.04, Debian 12 and 13, and AlmaLinux, Rocky Linux and RHEL 9 and 10 (this is ${PRETTY_NAME:-unknown})" ;;
esac
if [ "$FAMILY" = rhel ]; then PG_BIN=/usr/pgsql-18/bin; PG_SERVICE=postgresql-18; fi
[ "$(uname -m)" = x86_64 ] || [ "$(uname -m)" = aarch64 ] || die "x86_64 or aarch64 only"
# Resource limits of rootless containers need cgroup v2 (the default on every system above).
[ "$(stat -fc %T /sys/fs/cgroup 2> /dev/null)" = cgroup2fs ] || die "cgroup v2 is not in use (the unified hierarchy): hosting accounts' memory and CPU limits would not work"
command -v systemctl > /dev/null || die "systemd is required"
[ -n "$BIN_DIR" ] || die "give the release's binaries with --bin-dir DIR"
[ -n "$HOSTNAME_FQDN" ] || die "give the panel's DNS name with --hostname NAME"
[[ "$HOSTNAME_FQDN" =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$ ]] || die "--hostname must be a plain DNS name (lowercase letters, digits, dots, dashes)"
[ -n "$WEB_DIR" ] || { [ -d "$HERE/web" ] && WEB_DIR="$HERE/web"; } || true
if [ -n "$WEB_DIR" ]; then
    if [ -f "$WEB_DIR/web-client" ] && [ -f "$WEB_DIR/web-admin" ]; then WEB_FORM=archives
    elif [ -f "$WEB_DIR/client/index.html" ] && [ -f "$WEB_DIR/admin/index.html" ]; then WEB_FORM=folders
    else die "$WEB_DIR must hold web-client and web-admin (a release's archives) or client/index.html and admin/index.html (the built folders)"; fi
    [ -n "$ADMIN_HOSTNAME" ] || ADMIN_HOSTNAME="admin.$HOSTNAME_FQDN"
    [[ "$ADMIN_HOSTNAME" =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$ ]] || die "--admin-hostname must be a plain DNS name"
fi
[ -n "$ADMIN_EMAIL" ] || die "give the first administrator's email with --admin-email"
[[ "$ADMIN_EMAIL" =~ ^[^@\ ]+@[^@\ ]+\.[^@\ ]+$ ]] || die "--admin-email does not look like an email address"
need=(builtpanel-control-api bpctl); [ "$WITH_AGENT" -eq 1 ] && need+=(builtpanel-agent builtpanel-priv-helper); [ "$ROOT_LOGIN" -eq 1 ] && need+=(builtpanel-root-auth)
for b in "${need[@]}"; do
    [ -x "$BIN_DIR/$b" ] || die "$BIN_DIR/$b is missing or not executable"
    # An ELF program for this machine's architecture (the control plane has no --version: running
    # it would start the server).
    [ "$(head -c 4 "$BIN_DIR/$b" | od -An -c | tr -d ' ')" = '177ELF' ] || die "$BIN_DIR/$b is not a Linux program"
    machine=$(od -An -tx1 -j18 -N2 "$BIN_DIR/$b" | tr -d ' ')
    case "$(uname -m):$machine" in x86_64:3e00 | aarch64:b700) ;; *) die "$BIN_DIR/$b was built for another architecture ($machine)" ;; esac
done
free_gb=$(df -BG --output=avail / | tail -1 | tr -dc '0-9'); [ "$free_gb" -ge 10 ] || die "at least 10 GB free on / are needed (there are ${free_gb})"
mem_mb=$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo); [ "$mem_mb" -ge 1800 ] || die "at least 2 GB of memory are needed (there are ${mem_mb} MB)"

if [ "$CHECK" -eq 1 ]; then
    echo "==> check only: nothing is changed"
    echo "    system:        ${PRETTY_NAME} ($(uname -m)), ${mem_mb} MB memory, ${free_gb} GB free"
    echo "    system update: $([ "$UPGRADE" -eq 1 ] && echo 'every package and the kernel would be brought up to date first (--no-upgrade skips it)' || echo 'skipped (--no-upgrade)')"
    echo "    binaries:      ${need[*]} found and runnable"
    echo "    postgresql 18: $(command -v $PG_BIN/postgres > /dev/null && echo present || echo 'would be installed')"
    echo "    redis:         $(command -v redis-server || echo 'would be installed')"
    echo "    caddy:         $(command -v caddy || echo 'would be installed')"
    echo "    web interface: $([ -n "$WEB_DIR" ] && echo "$WEB_DIR (clients on $HOSTNAME_FQDN, administrators on $ADMIN_HOSTNAME)" || echo 'none given: only the API is served')"
    echo "    panel user:    $(id builtpanel > /dev/null 2>&1 && echo present || echo 'would be created')"
    echo "    configuration: $([ -f $ETC/control.toml ] && echo "present ($ETC/control.toml is kept)" || echo 'would be written')"
    echo "    CA:            $([ -f $SECRETS/node_ca.crt ] && echo 'present (kept)' || echo 'would be created')"
    echo "    root login:    $([ "$ROOT_LOGIN" -eq 1 ] && echo "on (root's password: $(passwd -S root 2> /dev/null | awk '{print $2}'))" || echo off)"
    exit 0
fi
[ "$(id -u)" -eq 0 ] || die "run as root (sudo)"
# From here the commands' own output goes to the log; the terminal shows the steps.
ui_log_to /var/log/builtpanel-install.log

export DEBIAN_FRONTEND=noninteractive
apt_install() { apt-get install -y -qq "$@" > /dev/null; }
dnf_install() {
    # A mirror can fail for a moment: try three times before giving up.
    local n
    for n in 1 2 3; do
        dnf install -y -q --setopt=retries=5 --setopt=timeout=60 "$@" > /dev/null && return 0
        sleep 5
    done
    return 1
}
MAJOR="${VERSION_ID%%.*}"
ARCH_RPM="$(uname -m)"

# --- the operating system ------------------------------------------------------------------------
# Brought up to date first: security fixes, and the kernel the rootless containers rely on. Nothing is
# restarted or rebooted here (needrestart only lists); a new kernel is used after the next reboot,
# which the last message asks for.
if [ "$UPGRADE" -eq 1 ]; then
    say "system update"
    if [ "$FAMILY" = rhel ]; then
        for n in 1 2 3; do
            dnf upgrade -y -q --refresh --setopt=retries=5 --setopt=timeout=60 > /dev/null && break
            [ "$n" -lt 3 ] || die "dnf upgrade failed three times (see the log); fix the repositories, or run again with --no-upgrade"
            sleep 5
        done
        # A newer kernel than the running one is installed: a reboot brings it in.
        newest_kernel=$(rpm -q kernel-core --qf '%{VERSION}-%{RELEASE}.%{ARCH}\n' --last 2> /dev/null | head -1) || newest_kernel=""
        [ -z "$newest_kernel" ] || [ "$newest_kernel" = "$(uname -r)" ] || NEED_REBOOT=1
    else
        # A new machine often runs its own first update at boot: wait for the package lock (up to 5 minutes).
        # The files the administrator edited are kept; full-upgrade, because a new kernel is a new package.
        export NEEDRESTART_MODE=l NEEDRESTART_SUSPEND=1
        apt_opts=(-o DPkg::Lock::Timeout=300 -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold)
        apt-get "${apt_opts[@]}" update -qq > /dev/null
        apt-get "${apt_opts[@]}" full-upgrade -y -qq > /dev/null
        apt-get "${apt_opts[@]}" autoremove -y -qq > /dev/null || true
        [ ! -e /run/reboot-required ] || NEED_REBOOT=1
    fi
fi

# --- packages --------------------------------------------------------------------------------
say "base packages"
if [ "$FAMILY" = rhel ]; then
    # curl is there already (curl-minimal is enough); the others: the firewall, the user-id mapping
    # tools and the user's own message bus for rootless Docker, slirp4netns for its network.
    dnf_install ca-certificates openssl firewalld shadow-utils dbus-daemon slirp4netns procps-ng policycoreutils policycoreutils-python-utils iptables-nft kmod
    command -v curl > /dev/null || dnf_install curl
    # Rootless containers need user namespaces (some images ship the limit at 0).
    if [ "$(sysctl -n user.max_user_namespaces 2> /dev/null || echo 1)" = 0 ]; then
        echo 'user.max_user_namespaces = 28633' > /etc/sysctl.d/99-builtpanel.conf
        sysctl -q -p /etc/sysctl.d/99-builtpanel.conf
    fi
    # Docker's network rules (iptables on nftables) use kernel modules that Red Hat 10 keeps in
    # kernel-modules-extra: without xt_addrtype the daemon refuses to start.
    if ! dnf_install "kernel-modules-extra-$(uname -r)" 2> /dev/null; then
        # The repositories no longer carry the running kernel's version: install the current kernel with
        # its extra modules; they are usable after one reboot.
        dnf_install kernel kernel-modules-extra || true
        NEED_REBOOT=1
    fi
    for m in nf_tables nft_compat xt_addrtype xt_conntrack xt_MASQUERADE xt_mark xt_comment br_netfilter overlay veth; do
        modprobe "$m" 2> /dev/null || true
    done
    printf 'xt_addrtype\nxt_conntrack\nxt_MASQUERADE\nxt_mark\nxt_comment\nnft_compat\nbr_netfilter\noverlay\nveth\n' > /etc/modules-load.d/builtpanel.conf
    if [ "$WITH_AGENT" -eq 1 ] && ! modprobe -n xt_addrtype > /dev/null 2>&1; then
        if [ "${NEED_REBOOT:-0}" = 1 ]; then
            die "a newer kernel was installed (Docker's network rules need its modules): run  reboot  and then this same command again"
        fi
        die "the kernel ($(uname -r)) has no xt_addrtype module and kernel-modules-extra is not available for it"
    fi
    # The web server (Caddy) may talk to the control plane and to the sites' containers on this machine.
    if command -v getenforce > /dev/null && [ "$(getenforce)" = Enforcing ]; then setsebool -P httpd_can_network_connect 1 || true; fi
else
    apt-get update -qq > /dev/null
    # slirp4netns: rootless Docker's network driver (newer Docker releases no longer bring it along).
    apt_install ca-certificates curl gnupg openssl ufw uidmap dbus-user-session slirp4netns
fi

say "PostgreSQL 18"
if [ ! -x "$PG_BIN/postgres" ]; then
    if [ "$FAMILY" = rhel ]; then
        dnf_install "https://download.postgresql.org/pub/repos/yum/reporpms/EL-${MAJOR}-${ARCH_RPM}/pgdg-redhat-repo-latest.noarch.rpm"
        # PGDG publishes one folder per major (rhel-10-...), not per point release (10.1, 10.3), and
        # dnf on EL10 cannot check the repository signature of its metadata: the packages stay signed.
        sed -i -e "s/\\\$releasever_major\\.\\\$releasever_minor/${MAJOR}/g" -e "s/\\\$releasever/${MAJOR}/g" \
            -e 's/^repo_gpgcheck *=.*/repo_gpgcheck=0/' /etc/yum.repos.d/pgdg-redhat-all.repo
        dnf clean metadata > /dev/null 2>&1 || true
        # The distribution's own (older) PostgreSQL module would hide the PGDG packages.
        dnf -qy module disable postgresql > /dev/null 2>&1 || true
        dnf_install postgresql18-server
        [ -s /var/lib/pgsql/18/data/PG_VERSION ] || "$PG_BIN/postgresql-18-setup" initdb > /dev/null
        # The panel connects over 127.0.0.1 with a password: the default there may be `ident`.
        sed -i -E 's/^(host[[:space:]]+all[[:space:]]+all[[:space:]]+(127\.0\.0\.1\/32|::1\/128)[[:space:]]+)(ident|md5|password)[[:space:]]*$/\1scram-sha-256/' /var/lib/pgsql/18/data/pg_hba.conf
    else
        install -d -m 0755 /usr/share/keyrings
        curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor --yes -o /usr/share/keyrings/postgresql.gpg
        echo "deb [signed-by=/usr/share/keyrings/postgresql.gpg] https://apt.postgresql.org/pub/repos/apt ${CODENAME}-pgdg main" > /etc/apt/sources.list.d/pgdg.list
        apt-get update -qq > /dev/null
        apt_install postgresql-18
    fi
fi
export PATH="$PG_BIN:$PATH"
systemctl enable --now "$PG_SERVICE" > /dev/null 2>&1
for _ in $(seq 1 30); do runuser -u postgres -- psql -qAt -c 'select 1' > /dev/null 2>&1 && break; sleep 1; done
runuser -u postgres -- psql -qAt -c 'select 1' > /dev/null 2>&1 || die "PostgreSQL does not answer (journalctl -u $PG_SERVICE)"

say "Redis"
if ! command -v redis-server > /dev/null && ! command -v valkey-server > /dev/null; then
    # The panel needs Redis 6.2 or newer (GETDEL). Ubuntu 22.04 ships 6.0, so there Redis is taken
    # from the Redis project's own repository; the other systems' own packages are new enough.
    if [ "$FAMILY" = rhel ]; then
        # Redis 6.2 or newer is in AppStream (9) or EPEL; Red Hat 10 has Valkey (Redis-compatible) instead.
        dnf_install redis 2> /dev/null || { dnf_install epel-release && dnf_install redis; } 2> /dev/null || dnf_install valkey
    elif [ "${ID}:${VERSION_ID}" = ubuntu:22.04 ]; then
        curl -fsSL https://packages.redis.io/gpg | gpg --dearmor --yes -o /usr/share/keyrings/redis-archive-keyring.gpg
        echo "deb [signed-by=/usr/share/keyrings/redis-archive-keyring.gpg] https://packages.redis.io/deb ${CODENAME} main" > /etc/apt/sources.list.d/redis.list
        apt-get update -qq > /dev/null
    fi
    if [ "$FAMILY" = deb ]; then apt_install redis-server; fi
fi
REDIS_BIN=redis-server
if [ "$FAMILY" = rhel ]; then
    # Red Hat 10 ships Valkey (the Redis-compatible server) instead of Redis.
    if ! command -v redis-server > /dev/null && command -v valkey-server > /dev/null; then REDIS_BIN=valkey-server; REDIS_SERVICE=valkey; REDIS_CONF=/etc/valkey/valkey.conf; else REDIS_SERVICE=redis; fi
fi
command -v "$REDIS_BIN" > /dev/null || die "Redis could not be installed (the log above has the reason)"
redis_version="$("$REDIS_BIN" --version | sed -n 's/.*v=\([0-9.]*\).*/\1/p')"
[ "$(printf '%s\n6.2\n' "$redis_version" | sort -V | head -1)" = 6.2 ] || die "Redis $redis_version is too old: the panel needs 6.2 or newer"
systemctl enable --now "$REDIS_SERVICE" > /dev/null 2>&1

say "Caddy"
if ! command -v caddy > /dev/null; then
    if [ "$FAMILY" = rhel ]; then
        # Caddy's own packages (the COPR the Caddy project publishes).
        dnf_install 'dnf-command(copr)'
        dnf copr enable -y @caddy/caddy > /dev/null
        dnf_install caddy
    else
        curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --dearmor --yes -o /usr/share/keyrings/caddy.gpg
        echo "deb [signed-by=/usr/share/keyrings/caddy.gpg] https://dl.cloudsmith.io/public/caddy/stable/deb/debian any-version main" > /etc/apt/sources.list.d/caddy.list
        apt-get update -qq > /dev/null
        apt_install caddy
    fi
fi

# --- user, folders, secrets ----------------------------------------------------------------
say "the panel's user and folders"
id builtpanel > /dev/null 2>&1 || useradd --system --user-group --home-dir "$STATE" --no-create-home --shell /usr/sbin/nologin builtpanel
install -d -m 0755 -o root -g builtpanel "$ETC"
install -d -m 0750 -o root -g builtpanel "$SECRETS"
install -d -m 0700 -o builtpanel -g builtpanel "$STATE"
for b in builtpanel-control-api bpctl; do install -m 0755 -o root -g root "$BIN_DIR/$b" /usr/local/bin/$b; done
[ "$ROOT_LOGIN" -eq 0 ] || install -m 0755 -o root -g root "$BIN_DIR/builtpanel-root-auth" /usr/local/bin/builtpanel-root-auth

secret() { # name, command that prints it: written once, never replaced
    if [ ! -s "$SECRETS/$1" ]; then
        (umask 077; "${@:2}" > "$SECRETS/$1")
    fi
    # The panel's own user reads it and nobody else: the program refuses a secret file that
    # anyone but its owner can read.
    chown builtpanel:builtpanel "$SECRETS/$1"; chmod 0600 "$SECRETS/$1"
}
say "database and Redis passwords"
secret db_password bash -c 'openssl rand -hex 24'
secret redis_password bash -c 'openssl rand -hex 24'
secret master_key bash -c 'openssl rand -base64 32'
DBPW=$(cat $SECRETS/db_password); RPW=$(cat $SECRETS/redis_password)

say "the panel's database"
runuser -u postgres -- psql -qAt -v ON_ERROR_STOP=1 -v pw="$DBPW" <<'SQL' > /dev/null
SELECT 'CREATE ROLE builtpanel LOGIN' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'builtpanel') \gexec
ALTER ROLE builtpanel PASSWORD :'pw';
SELECT 'CREATE DATABASE builtpanel OWNER builtpanel' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'builtpanel') \gexec
SQL
secret database_url printf 'postgres://builtpanel:%s@127.0.0.1/builtpanel' "$DBPW"

say "Redis password (local only)"
if ! grep -q "^requirepass" "$REDIS_CONF"; then
    printf '\nbind 127.0.0.1 -::1\nrequirepass %s\n' "$RPW" >> "$REDIS_CONF"
    systemctl restart "$REDIS_SERVICE"
fi
secret redis_url printf 'redis://:%s@127.0.0.1:6379/' "$RPW"

# --- configuration and the CA ---------------------------------------------------------------
say "configuration"
if [ ! -f "$ETC/control.toml" ]; then
    cat > "$ETC/control.toml" <<TOML
listen = "127.0.0.1:8080"
trusted_proxies = ["127.0.0.1", "::1"]

[log]
format = "json"
filter = "info"

[database]
url_file = "$SECRETS/database_url"
max_connections = 20

[redis]
url_file = "$SECRETS/redis_url"

[secrets]
master_key_file = "$SECRETS/master_key"

[pki]
ca_dir = "$SECRETS"

[agent]
enrollment_token_minutes = 15
public_url = "https://$HOSTNAME_FQDN"
stream_listen = "0.0.0.0:$AGENT_PORT"
stream_url = "https://$HOSTNAME_FQDN:$AGENT_PORT"
TOML
    chown root:builtpanel "$ETC/control.toml"; chmod 0640 "$ETC/control.toml"
fi
# Root login (like WHM): the control plane asks the root helper on this socket. Kept in step with
# --no-root-login on every run, also in a configuration written by an earlier run.
ROOT_SOCKET=/run/builtpanel-root-auth/root-auth.sock
sed -i '/^root_login_socket *=/d' "$ETC/control.toml"
if [ "$ROOT_LOGIN" -eq 1 ]; then
    if grep -q '^\[auth\]' "$ETC/control.toml"; then
        sed -i "/^\[auth\]/a root_login_socket = \"$ROOT_SOCKET\"" "$ETC/control.toml"
    else
        printf '\n[auth]\nroot_login_socket = "%s"\n' "$ROOT_SOCKET" >> "$ETC/control.toml"
    fi
fi
export BUILTPANEL_CONFIG="$ETC/control.toml"
as_panel() { runuser -u builtpanel -- env PATH="$PATH" BUILTPANEL_CONFIG="$BUILTPANEL_CONFIG" "$@"; }

say "the node channel's certificate authority"
# Made by root (the folder is not writable for the panel's user); the keys are handed over below.
[ -f "$SECRETS/node_ca.crt" ] || bpctl ca init --dir "$SECRETS" > /dev/null
[ -f "$SECRETS/server.crt" ] || bpctl ca issue-server --dir "$SECRETS" --dns "$HOSTNAME_FQDN" --out-dir "$SECRETS" > /dev/null
chmod 0750 "$SECRETS"; chmod 0640 "$SECRETS"/*.crt; chmod 0600 "$SECRETS"/*.key
# Made by root, read by the panel's user (the certificates are not secret, the keys are).
chown builtpanel:builtpanel "$SECRETS"/*.crt "$SECRETS"/*.key

say "database schema"
as_panel bpctl migrate > /dev/null

CPU_LIMITS=yes
ROOT_PW_STATE=""
if [ "$ROOT_LOGIN" -eq 1 ]; then
    say "root login (root with this server's root password)"
    install -m 0644 "$HERE/systemd/builtpanel-root-auth.service" /etc/systemd/system/builtpanel-root-auth.service
    systemctl daemon-reload
    systemctl enable builtpanel-root-auth > /dev/null 2>&1
    systemctl restart builtpanel-root-auth
    as_panel bpctl admin root-login --email "root@$HOSTNAME_FQDN" > /dev/null
    # P: root has a usable password; L: locked; NP: none (cloud images often log in with a key only).
    ROOT_PW_STATE=$(passwd -S root 2> /dev/null | awk '{print $2}')
    [ "$ROOT_PW_STATE" != PS ] || ROOT_PW_STATE=P   # the Red Hat family's passwd prints PS
else
    as_panel bpctl admin root-login --disable > /dev/null
    if [ -f /etc/systemd/system/builtpanel-root-auth.service ]; then
        systemctl disable --now builtpanel-root-auth > /dev/null 2>&1 || true
    fi
fi

# --- the service, the web server, the firewall ----------------------------------------------
say "the control plane's service"
install -m 0644 "$HERE/systemd/builtpanel-control-api.service" /etc/systemd/system/builtpanel-control-api.service
if [ "$FAMILY" = rhel ]; then
    # The unit names the Debian services; start after this system's own (postgresql-18, redis or valkey).
    install -d -m 0755 /etc/systemd/system/builtpanel-control-api.service.d
    printf '[Unit]\nAfter=%s.service %s.service\nWants=%s.service %s.service\n' "$PG_SERVICE" "$REDIS_SERVICE" "$PG_SERVICE" "$REDIS_SERVICE" > /etc/systemd/system/builtpanel-control-api.service.d/services.conf
fi
systemctl daemon-reload
systemctl enable builtpanel-control-api > /dev/null 2>&1
systemctl restart builtpanel-control-api
for _ in $(seq 1 40); do curl -fsS http://127.0.0.1:8080/api/v1/public/ready > /dev/null 2>&1 && break; sleep 0.5; done
curl -fsS http://127.0.0.1:8080/api/v1/public/ready > /dev/null 2>&1 || die "the control plane does not answer (journalctl -u builtpanel-control-api)"

WEB_ROOT=/usr/share/builtpanel/web
if [ -n "$WEB_DIR" ]; then
    say "the web interface (clients on $HOSTNAME_FQDN, administrators on $ADMIN_HOSTNAME)"
    install -d -m 0755 "$WEB_ROOT"
    for app in client admin; do
        rm -rf "$WEB_ROOT/$app.new"
        if [ "$WEB_FORM" = archives ]; then
            mkdir "$WEB_ROOT/$app.new"
            tar -xzf "$WEB_DIR/web-$app" -C "$WEB_ROOT/$app.new" --no-same-owner --no-same-permissions
        else
            cp -r "$WEB_DIR/$app" "$WEB_ROOT/$app.new"
        fi
        [ -f "$WEB_ROOT/$app.new/index.html" ] || die "the $app web interface has no index.html"
        chown -R root:root "$WEB_ROOT/$app.new"; chmod -R u=rwX,go=rX "$WEB_ROOT/$app.new"
        rm -rf "$WEB_ROOT/$app.old"; [ -d "$WEB_ROOT/$app" ] && mv "$WEB_ROOT/$app" "$WEB_ROOT/$app.old"
        mv "$WEB_ROOT/$app.new" "$WEB_ROOT/$app"; rm -rf "$WEB_ROOT/$app.old"
    done
    # SELinux: the web server reads files of this type.
    if command -v getenforce > /dev/null && [ "$(getenforce)" != Disabled ]; then
        chcon -R -t httpd_sys_content_t "$WEB_ROOT" > /dev/null 2>&1 || true
    fi
fi

# One site block of the panel: the API goes to the control plane, the rest is an app's files (any
# path that is not a file is the app's own page: it is a single-page app).
panel_site() { # name, app folder, [extra line inside the block]
    cat <<BLOCK
$1 {
    ${3:-}
    encode zstd gzip
    header {
        X-Content-Type-Options nosniff
        X-Frame-Options DENY
        Referrer-Policy strict-origin-when-cross-origin
    }
    handle /api/* {
        reverse_proxy 127.0.0.1:8080
    }
    handle {
        root * $WEB_ROOT/$2
        @assets path /assets/*
        header @assets Cache-Control "public, max-age=31536000, immutable"
        @page not path /assets/*
        header @page Cache-Control no-cache
        try_files {path} /index.html
        file_server
    }
}
BLOCK
}

# The server's public address (for the certificate-free way in), from the machine itself or, behind
# NAT, from an echo service; empty when neither gives an IPv4 address.
CLIENT_PORT=2083; ADMIN_PORT=2087
PUBLIC_IP=$(curl -4 -fsS --max-time 5 https://api.ipv4.icanhazip.com 2> /dev/null | tr -d ' \r\n' || true)
[[ "$PUBLIC_IP" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || PUBLIC_IP=$(ip -4 route get 1.1.1.1 2> /dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -1)
[[ "$PUBLIC_IP" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || PUBLIC_IP=""
if [ -n "$PUBLIC_IP" ] && [ "$FAMILY" = rhel ] && command -v semanage > /dev/null && [ "$(getenforce 2> /dev/null)" = Enforcing ]; then
    for port in $CLIENT_PORT $ADMIN_PORT; do semanage port -a -t http_port_t -p tcp "$port" 2> /dev/null || semanage port -m -t http_port_t -p tcp "$port" 2> /dev/null || true; done
fi

say "Caddy in front of the panel"
{
    options=""
    if [ "$WITH_AGENT" -eq 1 ]; then
        # One web server for the panel and for the sites on this machine. The node agent owns the
        # server named `builtpanel` (it fills it with every site's route), so the panel's own server has
        # that name and the agent is told the panel's names: they stay in what it sends.
        printf '{\n    servers :443 {\n        name builtpanel\n    }\n}\n'
    fi
    if [ -n "$WEB_DIR" ]; then
        panel_site "$HOSTNAME_FQDN" client
        panel_site "$ADMIN_HOSTNAME" admin
        # Also by the server's address, with a certificate of Caddy's own (the browser warns once):
        # the panel stays reachable when no real certificate can be had (a name that does not point
        # here yet, or Let's Encrypt's rate limit). Ports as in cPanel/WHM: 2083 clients, 2087 administrators.
        if [ -n "$PUBLIC_IP" ]; then
            panel_site "https://$PUBLIC_IP:$CLIENT_PORT" client "tls internal"
            panel_site "https://$PUBLIC_IP:$ADMIN_PORT" admin "tls internal"
        fi
    else
        printf '%s {\n    encode zstd gzip\n    reverse_proxy 127.0.0.1:8080\n}\n' "$HOSTNAME_FQDN"
    fi
} > /etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile > /dev/null 2>&1 || die "the Caddyfile is not valid (caddy validate --config /etc/caddy/Caddyfile)"
systemctl enable caddy > /dev/null 2>&1
systemctl reload caddy 2> /dev/null || systemctl restart caddy

if [ "$FAMILY" = rhel ]; then
    say "firewall (firewalld: 22, 80, 443, $AGENT_PORT)"
    systemctl enable --now firewalld > /dev/null 2>&1
    # ssh first: a server must never be locked out by this step.
    for svc in ssh http https; do firewall-cmd --permanent --add-service="$svc" > /dev/null; done
    firewall-cmd --permanent --add-port="$AGENT_PORT"/tcp > /dev/null
    [ -z "$PUBLIC_IP" ] || for port in $CLIENT_PORT $ADMIN_PORT; do firewall-cmd --permanent --add-port="$port"/tcp > /dev/null; done
    firewall-cmd --reload > /dev/null
else
    say "firewall (ufw: 22, 80, 443, $AGENT_PORT)"
    ufw allow 22/tcp > /dev/null; ufw allow 80/tcp > /dev/null; ufw allow 443/tcp > /dev/null; ufw allow "$AGENT_PORT"/tcp > /dev/null
    [ -z "$PUBLIC_IP" ] || { ufw allow "$CLIENT_PORT"/tcp > /dev/null; ufw allow "$ADMIN_PORT"/tcp > /dev/null; }
    ufw --force enable > /dev/null
fi

if [ "$WITH_AGENT" -eq 1 ]; then
    say "the local node (rootless Docker, the agent, the root helper)"
    if ! command -v dockerd-rootless-setuptool.sh > /dev/null; then
        if [ "$FAMILY" = rhel ]; then
            curl -fsSL https://download.docker.com/linux/centos/docker-ce.repo -o /etc/yum.repos.d/docker-ce.repo
            dnf_install docker-ce docker-ce-cli containerd.io docker-ce-rootless-extras
        else
            install -m 0755 -d /etc/apt/keyrings
            curl -fsSL "https://download.docker.com/linux/${ID}/gpg" | gpg --dearmor --yes -o /etc/apt/keyrings/docker.gpg
            echo "deb [signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/${ID} ${CODENAME} stable" > /etc/apt/sources.list.d/docker.list
            apt-get update -qq > /dev/null
            apt_install docker-ce docker-ce-cli containerd.io docker-ce-rootless-extras
        fi
        # Only the agent's own rootless daemon is used: the system-wide one stays off.
        systemctl disable --now docker.service docker.socket > /dev/null 2>&1 || true
    fi
    "$HERE/agent/install.sh" --binary "$BIN_DIR/builtpanel-agent" --priv-helper "$BIN_DIR/builtpanel-priv-helper" --rootless-docker
    agent_uid=$(id -u builtpanel-agent)
    if ! grep -qw cpu "/sys/fs/cgroup/user.slice/user-${agent_uid}.slice/user@${agent_uid}.service/cgroup.subtree_control" 2> /dev/null; then
        ui_warn "this server does not hand the CPU controller to the agent's user (a container-type VPS, or an old systemd): hosting accounts cannot get CPU limits here"
        CPU_LIMITS=no
    fi
    # The agent drives the web server on this machine (Caddy's admin API, which listens on this
    # machine only) and keeps the panel's own name in what it sends it.
    install -d -m 0755 /etc/systemd/system/builtpanel-agent.service.d
    cat > /etc/systemd/system/builtpanel-agent.service.d/panel.conf <<CONF
[Service]
Environment=BUILTPANEL_AGENT_CADDY_ADMIN=http://127.0.0.1:2019
Environment=BUILTPANEL_AGENT_PANEL_HOST=$HOSTNAME_FQDN
Environment=BUILTPANEL_AGENT_PANEL_UPSTREAM=127.0.0.1:8080
$([ -n "$WEB_DIR" ] && printf 'Environment=BUILTPANEL_AGENT_PANEL_ADMIN_HOST=%s\nEnvironment=BUILTPANEL_AGENT_PANEL_WEB_ROOT=%s\n' "$ADMIN_HOSTNAME" "$WEB_ROOT")
CONF
    systemctl daemon-reload
    systemctl try-restart builtpanel-agent 2> /dev/null || true
fi

# The administrator with a password of its own (it is the way in when `root` cannot be: root login
# off, or root has no password on this server). Made last, so a password that is printed is never
# lost to an install that stopped half way. With root login on and a root password set, root is
# the administrator, like WHM, and nothing is generated.
say "the first administrator"
ADMIN_PASSWORD=""
if [ "$ROOT_LOGIN" -eq 0 ] || [ "$ROOT_PW_STATE" != P ]; then
    if [ "$(runuser -u postgres -- psql -qAt -d builtpanel -c "select count(*) from accounts where kind = 'admin' and password_hash like '\$argon2id\$%'")" = 0 ]; then
        ADMIN_PASSWORD="$(openssl rand -base64 24 | tr -d '/+=' | cut -c1-24)-Aa1"
        printf '%s\n' "$ADMIN_PASSWORD" | as_panel bpctl admin create --username "$ADMIN_USER" --email "$ADMIN_EMAIL" --password-stdin > /dev/null
    fi
fi

say "the login message"
# Shown to whoever logs in to the server over SSH (a login shell on a terminal): what is installed,
# where to sign in, and the server's current load.
cat > /etc/profile.d/builtpanel-motd.sh <<MOTD
# Written by the Built Panel installer.
case "\$-" in *i*) ;; *) return 0 2> /dev/null || exit 0 ;; esac
[ -t 1 ] || return 0 2> /dev/null || exit 0
[ -z "\${BUILTPANEL_MOTD_SHOWN:-}" ] || return 0 2> /dev/null || exit 0
export BUILTPANEL_MOTD_SHOWN=1
_bp_cpu=\$(awk '/^cpu /{u=\$2+\$4; t=\$2+\$3+\$4+\$5+\$6+\$7+\$8; printf "%.1f", (t>0)?u*100/t:0}' /proc/stat 2> /dev/null)
_bp_ram=\$(awk '/^MemTotal/{t=\$2} /^MemAvailable/{a=\$2} END{printf "%d/%dMB (%.2f%%)", (t-a)/1024, t/1024, (t>0)?(t-a)*100/t:0}' /proc/meminfo 2> /dev/null)
_bp_disk=\$(df -P / 2> /dev/null | awk 'NR==2{printf "%d/%dGB (%s)", \$3/1048576, \$2/1048576, \$5}')
_bp_up=\$(awk '{d=int(\$1/86400); h=int((\$1%86400)/3600); m=int((\$1%3600)/60); printf "%d days, %d hours, %d minutes", d, h, m}' /proc/uptime 2> /dev/null)
printf '\nThis server has installed Built Panel.\n'
printf 'Log in  %s\n' '$([ -n "$PUBLIC_IP" ] && echo "https://$PUBLIC_IP:$ADMIN_PORT" || echo "https://$ADMIN_HOSTNAME")'
printf 'Login   root, with this server'"'"'s root password\n\n'
printf 'Current server time : %s\n' "\$(date '+%Y-%m-%d %H:%M:%S')"
printf 'Current load average: %s\n' "\$(cut -d' ' -f1-3 /proc/loadavg | sed 's/ /, /g')"
printf 'Current CPU usage   : %s%% (since boot)\n' "\$_bp_cpu"
printf 'Current RAM usage   : %s\n' "\$_bp_ram"
printf 'Current Disk usage  : %s\n' "\$_bp_disk"
printf 'System uptime       : %s\n\n' "\$_bp_up"
unset _bp_cpu _bp_ram _bp_disk _bp_up
MOTD
chmod 0644 /etc/profile.d/builtpanel-motd.sh

trap - EXIT
ui_done
cat -s <<EOF | ui_box
${UI_OK:-}${UI_B:-}Built Panel is installed.${UI_R:-}

  Panel:     https://$HOSTNAME_FQDN   (the certificate arrives within a minute, once the name points here)
$([ -n "$WEB_DIR" ] && printf '  Admin:     https://%s   (this name must point here too)\n' "$ADMIN_HOSTNAME")
$([ -n "$WEB_DIR" ] && [ -n "$PUBLIC_IP" ] && printf '  Without a certificate (works at once; the browser warns once: Advanced, continue):\n    admin:   https://%s:%s\n    clients: https://%s:%s\n' "$PUBLIC_IP" "$ADMIN_PORT" "$PUBLIC_IP" "$CLIENT_PORT")
$([ "$ROOT_LOGIN" -eq 1 ] && [ "$ROOT_PW_STATE" = P ] && printf '  Login:     root\n  Password:  this server'"'"'s root password (the one you log in to the server with)\n  Access:    full, like WHM'"'"'s root (role super_admin)\n')
$([ -n "$ADMIN_PASSWORD" ] && printf '  Login:     %s   (role super_admin: every permission on the panel)\n  Password:  %s\n  (shown once, never stored: sign in and change it, then set up two-factor)\n' "$ADMIN_USER" "$ADMIN_PASSWORD")
$([ "$ROOT_LOGIN" -eq 1 ] && [ "$ROOT_PW_STATE" != P ] && printf '  Root:      root has no usable password on this server: set one with `sudo passwd root`, then log in as root\n')
$([ -z "$ADMIN_PASSWORD" ] && [ "$ROOT_LOGIN" -eq 0 ] && echo '  (an administrator already existed; its password is unchanged)')

$([ "$CPU_LIMITS" = no ] && printf 'Note: this server does not hand the CPU controller to the agent, so hosting plans cannot limit CPU here.\n\n')Keep a copy of $SECRETS/master_key somewhere else: without it a lost server cannot be restored
(deploy/runbooks/system-restore.md).
$([ "${NEED_REBOOT:-0}" = 1 ] && printf '\nThe system update needs a reboot (a newer kernel or core libraries): run  reboot  when it suits you.\n')
$([ "$WITH_AGENT" -eq 1 ] && printf '\nThe local node is installed but not enrolled: add a node in the panel, then run the enrol command it\nshows as the agent user, then `systemctl enable --now builtpanel-agent` (see deploy/agent/install.sh).\n')
EOF
