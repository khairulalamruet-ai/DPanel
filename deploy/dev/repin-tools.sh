#!/usr/bin/env bash
# Moves the pinned third-party packages of deploy/install.sh (Kata Containers, Kiri) to their newest
# stable release and recomputes the SHA-256 of every file from the download itself. The release
# workflow runs it before the archive is built, so every release carries the newest tools while the
# installer still checks each download against a hash that is fixed inside the signed archive.
#
#   deploy/dev/repin-tools.sh           update deploy/install.sh in place
#   deploy/dev/repin-tools.sh --check   only report; exit 1 when a pin is behind
#
# A new major version (Kata) or minor version (Kiri, still 0.x) is never taken on its own: the layout
# of a package can change (Kata 4.x dropped the Go shim and `kata-runtime`), so a person looks first.
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 - "${1:-}" <<'PY'
import hashlib, json, re, sys, urllib.request
check = sys.argv[1] == "--check"
path = "deploy/install.sh"
text = open(path).read()
behind = False

def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": "builtpanel-repin"})
    return urllib.request.urlopen(req, timeout=120)

def sha256(url):
    h = hashlib.sha256()
    with get(url) as r:
        while chunk := r.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()

def vkey(v): return tuple(int(x) for x in v.split("."))

def pinned(name): return re.search(rf"^{name}=(\d+\.\d+\.\d+)$", text, re.M).group(1)

def apply(var, table, old, new, files):
    global text
    text = re.sub(rf"^{var}={re.escape(old)}$", f"{var}={new}", text, flags=re.M)
    block = re.search(rf"declare -A {table}=\(\n(?:    \[.*\n)+\)", text).group(0)
    lines = "".join(f'    ["{n}"]={h}\n' for n, h in files)
    text = text.replace(block, f"declare -A {table}=(\n{lines})")

# --- Kata Containers: the newest GitHub release; the digests come from the release itself ---------
old = pinned("KATA_VERSION")
rel = json.load(get("https://api.github.com/repos/kata-containers/kata-containers/releases/latest"))
new = rel["tag_name"].lstrip("v")
if new != old:
    if vkey(new)[0] != vkey(old)[0]:
        print(f"Kata Containers {new} is a new major version (pinned {old}): not taken automatically", file=sys.stderr)
    else:
        behind = True
        print(f"Kata Containers {old} -> {new}", file=sys.stderr)
        if not check:
            files = []
            for arch in ("amd64", "arm64"):
                name = f"kata-static-{new}-{arch}.tar.zst"
                digest = next((a.get("digest") for a in rel["assets"] if a["name"] == name), None)
                assert digest and digest.startswith("sha256:"), f"no digest for {name}"
                files.append((name, digest[7:]))
            apply("KATA_VERSION", "KATA_SHA256", old, new, files)
else:
    print(f"Kata Containers {old}: newest", file=sys.stderr)

# --- Kiri: the version the vendor's page announces; the hashes are computed from the downloads -----
old = pinned("KIRI_VERSION")
page = get("https://runkiri.com").read().decode("utf-8", "replace")
m = re.search(r"version>\s*(\d+\.\d+\.\d+)", page)
if not m:
    print("Kiri: the version could not be read from runkiri.com (pin kept)", file=sys.stderr)
else:
    new = m.group(1)
    if new == old:
        print(f"Kiri {old}: newest", file=sys.stderr)
    elif vkey(new)[:2] != vkey(old)[:2]:
        print(f"Kiri {new} is a new minor version (pinned {old}): not taken automatically", file=sys.stderr)
    elif vkey(new) < vkey(old):
        print(f"Kiri: the page says {new}, older than the pin {old}: pin kept", file=sys.stderr)
    else:
        behind = True
        print(f"Kiri {old} -> {new}", file=sys.stderr)
        if not check:
            names = [f"kiri-lite_{new}_amd64.deb", f"kiri-lite_{new}_arm64.deb",
                     f"kiri-lite-{new}-1.x86_64.rpm", f"kiri-lite-{new}-1.aarch64.rpm"]
            apply("KIRI_VERSION", "KIRI_SHA256", old, new, [(n, sha256(f"https://download.runkiri.com/{n}")) for n in names])

if not check:
    open(path, "w").write(text)
sys.exit(1 if (check and behind) else 0)
PY
