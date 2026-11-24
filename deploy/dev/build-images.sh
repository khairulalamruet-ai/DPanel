#!/usr/bin/env bash
# Builds our own images of the roles in deploy/images/* from the official image pinned in
# deploy/stack/images.toml (the first entry of the role), and scans the official image and ours
# with Trivy to show what the rebuild fixed. Run on the build box:
#   deploy/dev/remote.sh sh 'deploy/dev/build-images.sh [web db pma sftp]'
# Findings with no fix available yet are left out (--ignore-unfixed): only what can be fixed.
set -euo pipefail
cd "$(dirname "$0")/../.."
roles=("$@"); [ ${#roles[@]} -gt 0 ] || roles=(web db pma sftp)
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
TRIVY="docker run --rm -v $WORK:/work -v $HOME/.cache/trivy:/root/.cache aquasec/trivy:latest"
mkdir -p "$HOME/.cache/trivy"
base_of() { # role -> the first pinned reference of the role
    python3 - "$1" <<'PY'
import sys, tomllib
images = tomllib.load(open("deploy/stack/images.toml", "rb"))["image"]
print(next(i["reference"] for i in images if i["role"] == sys.argv[1]))
PY
}
count() { # image -> "CRITICAL HIGH" counts of fixable findings
    local target=("$1")
    case "$1" in builtpanel-*) # a local image is handed over as a file (Trivy does not reach the daemon)
        docker save "$1" -o "$WORK/image.tar"; target=(--input /work/image.tar) ;; esac
    $TRIVY image --quiet --scanners vuln --severity CRITICAL,HIGH --ignore-unfixed --format json "${target[@]}" 2>/dev/null |
        python3 -c 'import json,sys; d=json.load(sys.stdin); v=[x for r in d.get("Results",[]) for x in (r.get("Vulnerabilities") or [])]; print(sum(x["Severity"]=="CRITICAL" for x in v), sum(x["Severity"]=="HIGH" for x in v))'
}
for role in "${roles[@]}"; do
    base=$(base_of "$role")
    docker pull -q "$base" > /dev/null
    docker build -q --build-arg BASE="$base" -t "builtpanel-$role:test" "deploy/images/$role" > /dev/null
    read -r c0 h0 <<< "$(count "$base")"
    read -r c1 h1 <<< "$(count "builtpanel-$role:test")"
    printf '%-5s official: %s critical, %s high   ours: %s critical, %s high   (%s)\n' "$role" "$c0" "$h0" "$c1" "$h1" "${base%%@*}"
done
