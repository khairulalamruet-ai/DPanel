#!/usr/bin/env bash
# Build and test on a remote Linux box instead of the local machine.
#
# Syncs the current git worktree to the box and runs the checks there, so a
# slow or busy workstation is only used for editing.
#
#   deploy/dev/remote.sh rust [lint|test|deny|all]     backend checks (default: all)
#   deploy/dev/remote.sh web  [check|e2e]              frontend checks (default: check)
#   deploy/dev/remote.sh sh   '<command>'              run a command in the synced tree
#   deploy/dev/remote.sh status                        versions, disk and memory
#
# Settings come from the environment or from ~/.config/builtpanel/remote.env
# (never committed):
#   BUILTPANEL_REMOTE_HOST   user@host of the build box            (required)
#   BUILTPANEL_REMOTE_KEY    path to the SSH private key           (required)
#
# The box needs: rustup, PostgreSQL 18 and Redis for `rust`; Node and pnpm for
# `web`; and ~/.builtpanel-dev/secrets/{database_url,redis_test_url} (mode 0600).
# Each worktree gets its own directory (~/work/<worktree name>), so the two
# agents never overwrite each other; Rust build output is shared in
# ~/target-rust to save disk.
set -euo pipefail

CONFIG="${BUILTPANEL_REMOTE_CONFIG:-$HOME/.config/builtpanel/remote.env}"
# shellcheck disable=SC1090
[ -f "$CONFIG" ] && . "$CONFIG"
: "${BUILTPANEL_REMOTE_HOST:?set BUILTPANEL_REMOTE_HOST (or create $CONFIG)}"
: "${BUILTPANEL_REMOTE_KEY:?set BUILTPANEL_REMOTE_KEY (or create $CONFIG)}"

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
NAME="$(basename "$ROOT")"
REMOTE_DIR="work/$NAME"

remote() {
    ssh -i "$BUILTPANEL_REMOTE_KEY" -o IdentitiesOnly=yes -o BatchMode=yes \
        -o ServerAliveInterval=30 "$BUILTPANEL_REMOTE_HOST" "$@"
}

# Copies the worktree (tracked and untracked files, minus build output).
sync_tree() {
    remote "mkdir -p '$REMOTE_DIR'"
    rsync -az -e "ssh -i $BUILTPANEL_REMOTE_KEY" --delete \
        --exclude .git --exclude target --exclude node_modules --exclude dist \
        --exclude playwright-report --exclude test-results --exclude .claude \
        "$@" "$ROOT/" "$BUILTPANEL_REMOTE_HOST:$REMOTE_DIR/"
}

rust_checks() {
    local what="${1:-all}"
    case "$what" in lint | test | deny | all) ;; *) echo "unknown rust check: $what" >&2; exit 64 ;; esac
    sync_tree --exclude web
    remote "bash -s -- '$REMOTE_DIR' '$what'" <<'REMOTE'
set -uo pipefail
. "$HOME/.cargo/env"
cd "$HOME/$1"; what="$2"; rc=0
S="$HOME/.builtpanel-dev/secrets"
export DATABASE_URL="$(cat "$S/database_url")" REDIS_URL="$(cat "$S/redis_test_url")"
export SQLX_OFFLINE=true CARGO_TERM_COLOR=never CARGO_TARGET_DIR="$HOME/target-rust"
# The box has a small disk, and a full one stops PostgreSQL in the middle of a run. Before
# every run (and after it) what nobody needs is removed: incremental build data, unused Docker
# images and build cache, builds of our crates that are hours old. When space is short it goes
# further (every unused image; builds older than an hour), and when it is still short the run
# stops with a clear message instead of failing in strange ways.
free_gb() { df -BG --output=avail "$HOME" | tail -1 | tr -dc '0-9'; }
clean_disk() {
    rm -rf "$HOME/target-rust/debug/incremental"
    docker image prune -f > /dev/null 2>&1 || true
    docker builder prune -f > /dev/null 2>&1 || true
    find "$HOME/target-rust/debug/build" -mindepth 2 -maxdepth 2 -type d -mmin +360 -exec rm -rf {} + 2> /dev/null || true
    if [ "$(free_gb)" -lt 6 ]; then
        docker image prune -a -f > /dev/null 2>&1 || true
        docker volume prune -f > /dev/null 2>&1 || true
        find "$HOME/target-rust/debug/build" -mindepth 2 -maxdepth 2 -type d -mmin +60 -exec rm -rf {} + 2> /dev/null || true
    fi
    echo "disk: $(free_gb) GB free after cleaning"
}
clean_disk
if [ "$(free_gb)" -lt 3 ]; then
    echo "disk: less than 3 GB free even after cleaning; stopping (a full disk stops PostgreSQL)"
    exit 75
fi
step() { # name, command...
    local name="$1"; shift
    local log="$HOME/$name.log" start=$SECONDS
    if "$@" > "$log" 2>&1; then echo "$name: OK ($((SECONDS - start))s)"
    else echo "$name: FAILED ($((SECONDS - start))s) -- last lines:"; tail -25 "$log"; rc=1; fi
}
if [ "$what" = lint ] || [ "$what" = all ]; then
    step fmt cargo fmt --all --check
    step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
    step clippy-otel cargo clippy --workspace --all-targets --locked --features builtpanel-control-api/otel -- -D warnings
fi
if [ "$what" = test ] || [ "$what" = all ]; then
    # Live tests make Docker networks; a run that dies leaves them behind, and a few
    # dozen use up Docker's address pools ("all predefined address pools have been
    # fully subnetted"). Unused networks are of no use to anyone: remove them first.
    docker network prune -f > /dev/null 2>&1 || true
    # Two cores: with every test at once the database tests time out setting up (PoolTimedOut).
    # sqlx gives every test pool up to 5 connections out of ONE shared budget of 20, so at most 3
    # tests may run at once (3 x 5 = 15).
    step test env RUST_TEST_THREADS=3 cargo test --workspace --locked --no-fail-fast -- --include-ignored
    grep "test result" "$HOME/test.log" | awk '{p+=$4; f+=$6} END {print "tests: passed=" p " failed=" f}'
    grep -E "^test .* FAILED|panicked at" "$HOME/test.log" | head -20
fi
if [ "$what" = deny ] || [ "$what" = all ]; then
    step deny cargo deny check
fi
# Every edit leaves a new copy of our own crates and test programs in the shared
# target directory (different hash, same name). Keep the newest of each so a long
# session does not fill the disk; third-party crates are never touched.
python3 - <<'PRUNE'
import collections, os, re
deps = os.path.join(os.environ["CARGO_TARGET_DIR"], "debug", "deps")
groups = collections.defaultdict(list)
for name in os.listdir(deps):
    m = re.match(r"^(lib)?(.+?)-([0-9a-f]{16})(\.[a-z]+)?$", name)
    if not m:
        continue
    stem = (m.group(1) or "") + m.group(2)
    if "builtpanel_" in stem or "_live" in stem or stem.endswith("_schema"):
        groups[(stem, m.group(4) or "")].append((os.path.getmtime(os.path.join(deps, name)), name))
freed = 0
for items in groups.values():
    items.sort(reverse=True)
    for _, name in items[1:]:
        path = os.path.join(deps, name)
        freed += os.path.getsize(path)
        os.remove(path)
print(f"build cache: removed stale copies of our own crates, freed {freed / 1e9:.2f} GB")
PRUNE
clean_disk
df -h "$HOME" | awk 'NR==2 {print "disk: " $4 " free (" $5 " used)"}'
exit $rc
REMOTE
}

web_checks() {
    local what="${1:-check}"
    case "$what" in check | e2e) ;; *) echo "unknown web check: $what" >&2; exit 64 ;; esac
    [ -d "$ROOT/web" ] || { echo "no web/ directory in $ROOT" >&2; exit 66; }
    sync_tree --exclude '/apps' --exclude '/crates' --exclude '/migrations' --exclude '/.sqlx'
    remote "bash -s -- '$REMOTE_DIR' '$what'" <<'REMOTE'
set -uo pipefail
cd "$HOME/$1/web"; what="$2"; rc=0
export CI=true
step() {
    local name="$1"; shift
    local log="$HOME/web-$name.log" start=$SECONDS
    if "$@" > "$log" 2>&1; then echo "$name: OK ($((SECONDS - start))s)"
    else echo "$name: FAILED ($((SECONDS - start))s) -- last lines:"; tail -40 "$log"; rc=1; fi
}
step install pnpm install --frozen-lockfile
if [ $rc -eq 0 ]; then
    if [ "$what" = check ]; then
        step lint pnpm lint
        step typecheck pnpm typecheck
        step test pnpm test
        step build pnpm build
    else
        step playwright-install pnpm --filter @builtpanel/e2e exec playwright install --with-deps chromium
        step e2e pnpm test:e2e
    fi
fi
exit $rc
REMOTE
}

case "${1:-}" in
    rust) rust_checks "${2:-all}" ;;
    web) web_checks "${2:-check}" ;;
    sh)
        [ -n "${2:-}" ] || { echo "usage: remote.sh sh '<command>'" >&2; exit 64; }
        sync_tree
        remote "cd '$REMOTE_DIR' && [ -f ~/.cargo/env ] && . ~/.cargo/env; $2"
        ;;
    status)
        remote '. ~/.cargo/env 2>/dev/null; echo "rust:  $(rustc --version 2>/dev/null)"; echo "node:  $(node --version 2>/dev/null)  pnpm: $(pnpm --version 2>/dev/null)"; echo "pg:    $(psql --version 2>/dev/null)"; echo "redis: $(redis-server --version 2>/dev/null | cut -d" " -f3)"; echo "cpus:  $(nproc)"; free -h | sed -n 2,3p; df -h / | tail -1; ls ~/work 2>/dev/null | sed "s/^/worktree: /"'
        ;;
    *)
        sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
        exit 64
        ;;
esac
