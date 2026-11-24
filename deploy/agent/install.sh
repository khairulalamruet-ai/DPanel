#!/usr/bin/env bash
# Installs the Built Panel node agent on a Linux server (run as root).
#
#   sudo ./install.sh --binary ./builtpanel-agent [--priv-helper PATH | --no-priv-helper] [--engine podman|docker|kiri|docker-kata | --rootless-docker] [--check]
#
# The root helper is installed (or updated) automatically from the file called
# builtpanel-priv-helper next to the agent binary; --priv-helper PATH names another,
# --no-priv-helper leaves it out. Running this again after an upgrade replaces the
# binaries and the units and restarts the helper: nothing else to do by hand.
# --check only looks (what is there, what would be done) and changes nothing.
#
# What it does (safe to run again):
#   * creates the unprivileged system user `builtpanel-agent`
#   * installs the binary to /usr/local/bin and the hardened systemd unit
#   * creates /var/lib/builtpanel-agent (mode 0700, owned by the agent)
#   * installs and starts the small root helper the agent asks for the few host
#     changes that need root (its own hardened unit), and the packet filter tool
#     (nftables) it needs for the database's remote access, if it is missing
#   * with --engine podman: gives the agent its own rootless Podman (its Docker-compatible socket;
#     no daemon runs between requests, and the containers survive a restart of the agent)
#   * with --engine docker (or --rootless-docker): gives the agent its own rootless Docker daemon.
#     Docker and Podman give a tenant's files the same ids, so a node can change from one to the other.
#   * with --engine kiri: uses the Kiri engine (install the package first: install.sh of the panel does)
#     instead: the agent joins its group, the helper hands tenant files out under their own ids, and
#     the engine is switched on. Kiri is a root engine: a member of its `kiri` group is root-equivalent.
#     The engine of a node that already has tenants is not changed (their files belong to the old ids).
#   * with --engine docker-kata: Docker (the system's) with Kata Containers as the runtime of every container,
#     each in its own light virtual machine (the packages first: install.sh of the panel does). Like Kiri it is
#     a root engine: the agent joins the `docker` group, which is root-equivalent.
#
# It does NOT enrol or start the agent: enrol first, with the command the panel
# shows when you add the node (run as the agent user), then enable the service.
set -euo pipefail

export PATH="/usr/local/bin:/usr/local/sbin:/usr/sbin:/sbin:$PATH"   # sudo on the Red Hat family leaves these out
AGENT_USER=builtpanel-agent
STATE_DIR=/var/lib/builtpanel-agent
BIN_DST=/usr/local/bin/builtpanel-agent
UNIT_DST=/etc/systemd/system/builtpanel-agent.service
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

binary=""
helper=""
no_helper=0
check=0
rootless=0
engine=""
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) binary="${2:?--binary needs a path}"; shift 2 ;;
    --priv-helper) helper="${2:?--priv-helper needs a path}"; shift 2 ;;
    --no-priv-helper) no_helper=1; shift ;;
    --check) check=1; shift ;;
    --rootless-docker) rootless=1; shift ;;
    --engine) engine="${2:?--engine needs podman, docker or kiri}"; shift 2 ;;
    -h|--help) sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "install.sh: unknown option: $1" >&2; exit 64 ;;
  esac
done

die() { echo "install.sh: $*" >&2; exit 1; }
say() { echo "==> $*"; }

case "$engine" in "" | podman | docker | kiri | docker-kata) ;; *) die "--engine must be podman, docker, kiri or docker-kata" ;; esac
[ "$engine" != docker-kata ] || [ "$rootless" -eq 0 ] || die "--engine docker-kata and --rootless-docker exclude each other"
[ "$engine" != kiri ] || [ "$rootless" -eq 0 ] || die "--engine kiri and --rootless-docker exclude each other"
[ "$engine" != podman ] || [ "$rootless" -eq 0 ] || die "--engine podman and --rootless-docker exclude each other"
# From here on `rootless` means a rootless engine (Podman or Docker) is wanted.
[ "$engine" != podman ] && [ "$engine" != docker ] || rootless=1
case "$engine" in podman | kiri | docker-kata) ;; *) engine=docker ;; esac
ENGINE_FILE=/etc/builtpanel/node-engine

if [ "$check" -eq 0 ]; then
  [ "$(id -u)" -eq 0 ] || die "run as root (sudo)"
fi
[ "$(uname -s)" = Linux ] || die "Linux only"
command -v systemctl >/dev/null || die "systemd is required"
[ -n "$binary" ] || die "give the agent binary with --binary PATH"
[ -x "$binary" ] || die "$binary is not an executable file"
"$binary" --version >/dev/null 2>&1 || die "$binary does not run on this machine"

# The helper: the one named, or the one beside the agent binary.
if [ "$no_helper" -eq 1 ]; then
  helper=""
elif [ -z "$helper" ] && [ -x "$(dirname "$binary")/builtpanel-priv-helper" ]; then
  helper="$(dirname "$binary")/builtpanel-priv-helper"
fi

if [ "$check" -eq 1 ]; then
  say "check only: nothing is changed"
  say "agent binary:   $binary ($("$binary" --version 2>/dev/null | head -1))"
  if [ -n "$helper" ]; then
    "$helper" --version >/dev/null 2>&1 || die "$helper does not run on this machine"
    say "root helper:    $helper ($("$helper" --version 2>/dev/null | head -1)) would be installed"
  else
    say "root helper:    none (use --priv-helper PATH, or put builtpanel-priv-helper next to the agent binary)"
  fi
  say "nftables:       $(command -v nft || echo 'missing (would be installed)')"
  say "agent user:     $(id "$AGENT_USER" >/dev/null 2>&1 && echo present || echo 'would be created')"
  exit 0
fi

# --- the engine of this node -------------------------------------------------
# Tenant files are owned by ids that depend on the engine (a rootless engine's subordinate range, or the
# ids as they are for Kiri): a node that has tenants cannot change engine without handing every file over.
# Rootless Docker and rootless Podman use the same ids (the agent's subordinate range), so those two can
# change into each other (the containers are made again from what the panel knows; the files stay); Kiri
# is the other family.
if [ "$rootless" -eq 1 ] || [ "$engine" = kiri ] || [ "$engine" = docker-kata ]; then
  now_engine=rootless; [ "$engine" != kiri ] && [ "$engine" != docker-kata ] || now_engine=root
  had="$(cat "$ENGINE_FILE" 2>/dev/null || true)"
  if [ -z "$had" ] && [ -d /srv/tenants ] && [ -n "$(ls -A /srv/tenants 2>/dev/null)" ]; then
    had=docker   # tenants from before this file existed were made under rootless Docker
  fi
  case "$had" in docker | podman) had=rootless ;; kiri | docker-kata) had=root ;; esac
  if [ -n "$had" ] && [ "$had" != "$now_engine" ] && [ -n "$(ls -A /srv/tenants 2>/dev/null)" ]; then
    die "this node already has tenants under the $had engine: their files are owned by that engine's ids, so it cannot change to $now_engine (use a new server)"
  fi
fi

# --- user and directories ---------------------------------------------------
if id "$AGENT_USER" >/dev/null 2>&1; then
  say "user $AGENT_USER exists"
else
  say "creating user $AGENT_USER"
  # The home directory is inside the state directory so the rootless Docker
  # data of the agent lives (and stays private) with it.
  useradd --system --user-group --home-dir "$STATE_DIR/home" --no-create-home \
          --shell /usr/sbin/nologin "$AGENT_USER"
fi
AGENT_UID="$(id -u "$AGENT_USER")"
install -d -m 0700 -o "$AGENT_USER" -g "$AGENT_USER" "$STATE_DIR" "$STATE_DIR/home"
# Files on their way between the network and a tenant's files: the agent and (through its
# group) the privileged helper only.
install -d -m 0770 -o "$AGENT_USER" -g "$AGENT_USER" /var/lib/builtpanel-transfers

# --- binary and unit --------------------------------------------------------
say "installing $BIN_DST"
install -m 0755 -o root -g root "$binary" "$BIN_DST"
say "installing $UNIT_DST"
sed "s/@AGENT_UID@/${AGENT_UID}/g" "$HERE/builtpanel-agent.service" > "$UNIT_DST.new"
chmod 0644 "$UNIT_DST.new"
mv -f "$UNIT_DST.new" "$UNIT_DST"
if command -v systemd-analyze >/dev/null; then
  systemd-analyze verify "$UNIT_DST" 2>&1 | grep -v "^$" | sed 's/^/    /' || true
fi
systemctl daemon-reload

# --- privileged helper -------------------------------------------------------
if [ -n "$helper" ]; then
  [ -x "$helper" ] || die "$helper is not an executable file"
  "$helper" --version >/dev/null 2>&1 || die "$helper does not run on this machine"
  # The packet filter tool the helper uses for the database's remote access.
  if ! command -v nft >/dev/null 2>&1 && [ ! -x /usr/sbin/nft ]; then
    say "installing nftables"
    if command -v apt-get >/dev/null; then
      DEBIAN_FRONTEND=noninteractive apt-get install -y nftables >/dev/null
    elif command -v dnf >/dev/null; then
      dnf install -y nftables >/dev/null
    elif command -v yum >/dev/null; then
      yum install -y nftables >/dev/null
    elif command -v zypper >/dev/null; then
      zypper --non-interactive install nftables >/dev/null
    else
      die "nftables is missing and no known package manager was found: install it, then run this again"
    fi
  fi
  [ -x /usr/sbin/nft ] || die "nftables is installed but /usr/sbin/nft is not there (the helper runs it by that path)"
  HELPER_DST=/usr/local/bin/builtpanel-priv-helper
  HELPER_UNIT=/etc/systemd/system/builtpanel-priv-helper.service
  say "installing $HELPER_DST"
  install -m 0755 -o root -g root "$helper" "$HELPER_DST.new"
  mv -f "$HELPER_DST.new" "$HELPER_DST"
  install -m 0644 -o root -g root "$HERE/builtpanel-priv-helper.service" "$HELPER_UNIT"
  if [ "$engine" = kiri ] || [ "$engine" = docker-kata ]; then
    # Containers of a root engine have no user namespace of their own: a tenant's id N is the host's id N.
    install -d -m 0755 /etc/systemd/system/builtpanel-priv-helper.service.d
    printf '[Service]\nEnvironment=BUILTPANEL_HELPER_UID_MAP=identity\n' > /etc/systemd/system/builtpanel-priv-helper.service.d/engine.conf
  elif [ "$rootless" -eq 1 ]; then
    rm -f /etc/systemd/system/builtpanel-priv-helper.service.d/engine.conf
  fi
  systemctl daemon-reload
  # Where tenants live. It must exist before the unit starts (the unit's sandbox
  # lists it as writable); only root may create things in it.
  install -d -m 0755 -o root -g root /srv/tenants
  say "starting builtpanel-priv-helper"
  systemctl enable builtpanel-priv-helper >/dev/null 2>&1
  systemctl restart builtpanel-priv-helper
  for _ in $(seq 1 20); do
    [ -S /run/builtpanel/priv-helper.sock ] && break
    sleep 0.5
  done
  [ -S /run/builtpanel/priv-helper.sock ] || die "the privileged helper did not create its socket (journalctl -u builtpanel-priv-helper)"
else
  say "skipping the privileged helper (none given or found next to the agent binary)"
fi

# --- rootless Docker for the agent -------------------------------------------
if [ "$rootless" -eq 1 ]; then
  if [ "$engine" = podman ]; then
    command -v podman >/dev/null || die "podman not found: install the podman package (and uidmap) first"
  else
    command -v dockerd-rootless-setuptool.sh >/dev/null \
      || die "dockerd-rootless-setuptool.sh not found: install docker-ce-rootless-extras (and uidmap) first"
  fi
  command -v newuidmap >/dev/null || die "newuidmap not found: install the uidmap package"
  command -v slirp4netns >/dev/null || die "slirp4netns not found: install the slirp4netns package"
  command -v loginctl >/dev/null || die "loginctl not found"

  if ! grep -q "^${AGENT_USER}:" /etc/subuid 2>/dev/null; then
    # First free block of 65536 ids after everything already handed out.
    start=$(awk -F: 'BEGIN{m=100000} {e=$2+$3; if (e>m) m=e} END{print m}' /etc/subuid /etc/subgid 2>/dev/null)
    say "allocating subordinate ids ${start}-$((start + 65535)) for $AGENT_USER"
    usermod --add-subuids "${start}-$((start + 65535))" --add-subgids "${start}-$((start + 65535))" "$AGENT_USER"
  fi
  # A plan's CPU, memory and process limits are enforced by cgroup v2 controllers, which systemd
  # hands to a user's own services only when told to: older systemd versions (Ubuntu 22.04 has 249)
  # leave `cpu` out, and rootless Docker then refuses every CPU limit.
  say "delegating the cgroup controllers (cpu, cpuset, io, memory, pids) to users' services"
  install -d -m 0755 /etc/systemd/system/user@.service.d
  printf '[Service]\nDelegate=cpu cpuset io memory pids\n' > /etc/systemd/system/user@.service.d/builtpanel-delegate.conf
  systemctl daemon-reload
  say "enabling lingering so the agent's rootless engine runs without a login"
  loginctl enable-linger "$AGENT_USER"
  for _ in $(seq 1 30); do
    systemctl is-active --quiet "user@${AGENT_UID}.service" && break
    sleep 1
  done
  systemctl is-active --quiet "user@${AGENT_UID}.service" || die "user@${AGENT_UID}.service did not start"
  # A user manager that was already running does not know the new delegation. It has to start
  # again, and cgroup v2 lets it only when nothing of the old one is left in its cgroup.
  controllers="/sys/fs/cgroup/user.slice/user-${AGENT_UID}.slice/user@${AGENT_UID}.service/cgroup.subtree_control"
  if ! grep -qw cpu "$controllers" 2>/dev/null; then
    systemctl stop builtpanel-agent 2>/dev/null || true
    pkill -KILL -u "$AGENT_UID" 2>/dev/null || true
    sleep 2
    systemctl reset-failed "user@${AGENT_UID}.service" 2>/dev/null || true
    systemctl restart "user@${AGENT_UID}.service"
    for _ in $(seq 1 30); do systemctl is-active --quiet "user@${AGENT_UID}.service" && break; sleep 1; done
  fi
  # Not fatal: the panel and the node work, only a plan's CPU limit cannot be enforced (a
  # container-type VPS, or an old systemd, does not hand the controller over). install.sh says so.
  grep -qw cpu "$controllers" 2>/dev/null || echo "install.sh: warning: the cpu controller is not delegated to the agent's user ($controllers): CPU limits will not work" >&2

  as_agent() {
    # From `/`: the caller's directory may be one the agent cannot enter, and Podman re-executes itself in it.
    (cd / && runuser -u "$AGENT_USER" -- env HOME="$STATE_DIR/home" \
      XDG_RUNTIME_DIR="/run/user/${AGENT_UID}" \
      DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${AGENT_UID}/bus" \
      PATH="/usr/bin:/usr/sbin:/bin:/sbin" "$@")
  }
  if [ "$engine" = podman ]; then
    PODMAN_SOCK="/run/user/${AGENT_UID}/podman/podman.sock"
    say "setting up rootless Podman for $AGENT_USER"
    # The agent talks Podman's Docker-compatible API; the engine is a drop-in over the unit's default socket.
    install -d -m 0755 /etc/systemd/system/builtpanel-agent.service.d
    printf '[Service]\nEnvironment=DOCKER_HOST=unix://%s\n' "$PODMAN_SOCK" > /etc/systemd/system/builtpanel-agent.service.d/engine.conf
    systemctl daemon-reload
    # Podman has no daemon: the socket starts a service on demand. Containers it made keep running
    # without it; this unit starts the ones with a restart policy again after a reboot.
    # A node that ran rootless Docker: its containers stop (they hold the ports), the data kept in Docker's
    # own volumes (the database servers, webmail) is copied into Podman's with the ids unchanged, and the
    # containers are made again by the agent from what the panel knows (images are pulled again). Docker's
    # data stays where it is, so `--engine docker` can go back.
    docker_volumes="$STATE_DIR/home/.local/share/docker/volumes"
    if [ "$(cat "$ENGINE_FILE" 2>/dev/null || true)" != podman ] && [ -d "$docker_volumes" ]; then
      say "moving this node from rootless Docker to rootless Podman"
      agent_was_active=0
      if systemctl is-active --quiet builtpanel-agent; then agent_was_active=1; systemctl stop builtpanel-agent; fi
      as_agent systemctl --user stop docker.service docker.socket >/dev/null 2>&1 || true
      as_agent systemctl --user disable docker.service >/dev/null 2>&1 || true
      as_agent systemctl --user enable --now podman.socket >/dev/null 2>&1 || die "podman.socket did not start for $AGENT_USER"
      for vol in "$docker_volumes"/*/; do
        name="$(basename "$vol")"
        [ -d "$vol/_data" ] || continue
        [ ! -e "$vol/opts.json" ] || continue            # a bind volume: nothing of its own inside
        [ -n "$(ls -A "$vol/_data" 2>/dev/null)" ] || continue
        if as_agent podman volume exists "$name" 2>/dev/null; then
          say "volume $name is already in Podman: not copied again"
          continue
        fi
        say "copying volume $name"
        as_agent podman volume create --label builtpanel.managed=true "$name" >/dev/null
        dest="$(as_agent podman volume inspect "$name" --format '{{.Mountpoint}}')"
        # Inside the user namespace, so the copied files keep the ids the containers know.
        as_agent podman unshare cp -a "$vol/_data/." "$dest/" || die "copying volume $name failed (Docker's data is untouched)"
      done
      [ "$agent_was_active" -eq 0 ] || systemctl start builtpanel-agent || true
    fi
    as_agent systemctl --user enable --now podman.socket >/dev/null 2>&1 || die "podman.socket did not start for $AGENT_USER"
    as_agent systemctl --user enable podman-restart.service >/dev/null 2>&1 || true
    for _ in $(seq 1 30); do
      [ -S "$PODMAN_SOCK" ] && break
      sleep 1
    done
    [ -S "$PODMAN_SOCK" ] || die "the agent's Podman socket did not appear"
    [ "$(as_agent podman info --format '{{.Host.CgroupsVersion}}' 2>/dev/null)" = v2 ] || die "the agent's Podman does not run on cgroup v2: a plan's CPU, memory and process limits need it"
    say "rootless Podman is up: $(as_agent podman version --format '{{.Server.Version}}' 2>/dev/null | cut -c1-40)"
  else
  rm -f /etc/systemd/system/builtpanel-agent.service.d/engine.conf
  systemctl daemon-reload
  say "setting up rootless Docker for $AGENT_USER"
  as_agent dockerd-rootless-setuptool.sh install >/dev/null
  # The real address of a visitor must reach the containers (the SFTP server counts failed
  # logins per address, T11.07). Rootless Docker's default port driver (builtin) shows every
  # container the bridge gateway instead; slirp4netns passes the true source address on.
  dropin="$STATE_DIR/home/.config/systemd/user/docker.service.d"
  want=$'[Service]\nEnvironment=DOCKERD_ROOTLESS_ROOTLESSKIT_NET=slirp4netns\nEnvironment=DOCKERD_ROOTLESS_ROOTLESSKIT_PORT_DRIVER=slirp4netns\n'
  if [ "$(cat "$dropin/port-driver.conf" 2>/dev/null)" != "${want%$'\n'}" ]; then
    say "using the slirp4netns port driver so containers see real client addresses"
    as_agent mkdir -p "$dropin"
    printf '%s' "$want" | as_agent tee "$dropin/port-driver.conf" >/dev/null
    as_agent systemctl --user daemon-reload >/dev/null 2>&1 || true
    # A running daemon picks the driver up only on a restart (its containers are recreated
    # by the agent's next convergence); a fresh install is started below.
    as_agent systemctl --user is-active --quiet docker && as_agent systemctl --user restart docker >/dev/null 2>&1 || true
  fi
  as_agent systemctl --user enable --now docker >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    [ -S "/run/user/${AGENT_UID}/docker.sock" ] && break
    sleep 1
  done
  [ -S "/run/user/${AGENT_UID}/docker.sock" ] || die "the agent's Docker socket did not appear"
  # Docker itself says whether it can limit CPU and memory: a hosting plan needs both.
  if as_agent env DOCKER_HOST="unix:///run/user/${AGENT_UID}/docker.sock" docker info 2>&1 | grep -qiE "No (cpu cfs|memory limit|pids limit)"; then
    die "the agent's Docker cannot limit CPU, memory or processes (docker info warns): check the cgroup v2 delegation"
  fi
  say "rootless Docker is up: $(as_agent env DOCKER_HOST="unix:///run/user/${AGENT_UID}/docker.sock" docker version --format '{{.Server.Version}} (rootless={{range .Server.Components}}{{.Name}} {{end}})' 2>/dev/null | cut -c1-80)"
  fi
elif [ "$engine" = kiri ]; then
  command -v kiri >/dev/null || die "kiri is not installed (install the kiri-lite package, or let the panel's install.sh do it)"
  getent group kiri >/dev/null || die "the kiri group does not exist: is the kiri package installed?"
  say "using the Kiri engine for $AGENT_USER (a root engine: the agent is in its root-equivalent group)"
  # The agent reaches Kiri's Docker API through the kiri group and starts after the engine.
  install -d -m 0755 /etc/systemd/system/builtpanel-agent.service.d
  cat > /etc/systemd/system/builtpanel-agent.service.d/engine.conf <<'CONF'
[Unit]
After=kiri.service
Wants=kiri.service

[Service]
SupplementaryGroups=kiri
Environment=DOCKER_HOST=unix:///run/kiri/docker.sock
CONF
  systemctl daemon-reload
  systemctl enable --now kiri >/dev/null 2>&1 || die "kiri did not start (journalctl -u kiri)"
  for _ in $(seq 1 30); do [ -S /run/kiri/docker.sock ] && break; sleep 1; done
  [ -S /run/kiri/docker.sock ] || die "Kiri's Docker socket did not appear (journalctl -u kiri)"
  [ "$(curl -fsS --unix-socket /run/kiri/docker.sock http://localhost/_ping 2>/dev/null)" = OK ] || die "Kiri does not answer on its socket (journalctl -u kiri)"
  say "Kiri is up: $(curl -fsS --unix-socket /run/kiri/docker.sock http://localhost/version 2>/dev/null | sed -n 's/.*"KiriVersion":"\([^"]*\)".*/\1/p')"
elif [ "$engine" = docker-kata ]; then
  command -v dockerd >/dev/null || die "docker is not installed (install docker-ce, or let the panel's install.sh do it)"
  [ -x /usr/local/bin/containerd-shim-kata-v2 ] || die "Kata Containers is not installed (let the panel's install.sh put it in /opt/kata and link its shim)"
  [ -c /dev/kvm ] || die "/dev/kvm is missing: Kata Containers needs hardware virtualization"
  say "using Docker with Kata Containers for $AGENT_USER (a root engine: the agent is in the docker group, which is root-equivalent)"
  # Kata is a runtime of the system's Docker, found through containerd's shim name.
  install -d -m 0755 /etc/docker
  if [ ! -s /etc/docker/daemon.json ]; then
    printf '{\n  "runtimes": {\n    "kata": { "runtimeType": "io.containerd.kata.v2" }\n  }\n}\n' > /etc/docker/daemon.json
  elif ! grep -q '"kata"' /etc/docker/daemon.json; then
    die "/etc/docker/daemon.json exists without a \"kata\" runtime: add  \"runtimes\": { \"kata\": { \"runtimeType\": \"io.containerd.kata.v2\" } }  to it and run this again"
  fi
  systemctl enable docker >/dev/null 2>&1 || true
  systemctl restart docker >/dev/null 2>&1 || die "docker did not start (journalctl -u docker)"
  for _ in $(seq 1 30); do [ -S /run/docker.sock ] && break; sleep 1; done
  [ -S /run/docker.sock ] || die "Docker's socket did not appear (journalctl -u docker)"
  docker info 2>/dev/null | grep -qi 'Runtimes:.*kata' || die "Docker does not list the kata runtime (docker info)"
  # The proof that matters: a virtual machine really starts (no KVM, or no nested virtualization, fails here).
  say "starting a test container in a Kata virtual machine"
  if ! timeout 180 docker run --rm --runtime kata --network none docker.io/library/busybox:stable true >/dev/null 2>&1; then
    die "Kata could not start a virtual machine here (journalctl -u docker): is nested virtualization on?"
  fi
  install -d -m 0755 /etc/systemd/system/builtpanel-agent.service.d
  cat > /etc/systemd/system/builtpanel-agent.service.d/engine.conf <<'CONF'
[Unit]
After=docker.service
Wants=docker.service

[Service]
SupplementaryGroups=docker
Environment=DOCKER_HOST=unix:///run/docker.sock
Environment=BUILTPANEL_CONTAINER_RUNTIME=kata
CONF
  systemctl daemon-reload
  say "Docker with Kata is up: $(docker version --format '{{.Server.Version}}' 2>/dev/null | cut -c1-40)"
else
  say "skipping rootless Docker (use --rootless-docker to set it up for the agent)"
fi
# What engine this node was set up with (checked above on the next run).
if [ "$rootless" -eq 1 ] || [ "$engine" = kiri ] || [ "$engine" = docker-kata ]; then
  install -d -m 0755 /etc/builtpanel
  printf '%s\n' "$engine" > "$ENGINE_FILE"
fi

cat <<EOF

Installed.$([ -n "$helper" ] && printf '\n\nThe database server is reached from outside the node on port 13306 only by the\naddresses a client lists, but if this machine sits behind a provider firewall, let\nthat port through there too.')

Next:
  1. Enrol (once, as the agent user), using the command shown when you add the node in the panel:
       sudo -u $AGENT_USER builtpanel-agent enroll --server … --ca-fingerprint … --token …
  2. Start the service:
       sudo systemctl enable --now builtpanel-agent
  3. Check it:
       sudo -u $AGENT_USER builtpanel-agent status
       systemctl status builtpanel-agent
EOF
