#!/usr/bin/env python3
"""Finds the newest release of every container entry of the marketplace and pins it.

    deploy/marketplace/refresh.py --check      list what is newer than what the entries say
    deploy/marketplace/refresh.py --write      rewrite version, tag and digest in the files
    deploy/marketplace/refresh.py --write redis solr    only these

For each `kind = "container"` entry it asks the image's registry (the standard registry API, no
login: Docker Hub, quay.io, docker.elastic.co, ghcr.io) for the tags, keeps the plain version tags
(`8.0.2`, not `8.0.2-alpine`), keeps those that start with the entry's `track` (`8` stays in 8.x;
empty follows every release), takes the highest, and pins the digest of that tag. Review the
change, run the sandbox check on the build box (every image must start with no capabilities),
then ship: customers see "update available" and update from the marketplace.

Python 3 standard library only.
"""
import json
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ACCEPT = ", ".join([
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
])
VERSION = re.compile(r"^v?(\d+(?:\.\d+){0,3})(?:\.Final(?:-jdk21)?)?$")


def split_image(image):
    host, _, repo = image.partition("/")
    if host == "docker.io":
        return "registry-1.docker.io", repo
    return host, repo


class Registry:
    def __init__(self, host, repo):
        self.host, self.repo, self.token = host, repo, None

    def _get(self, path, method="GET", headers=None):
        url = f"https://{self.host}/v2/{self.repo}/{path}"
        for _ in range(2):
            h = dict(headers or {})
            if self.token:
                h["Authorization"] = "Bearer " + self.token
            req = urllib.request.Request(url, method=method, headers=h)
            try:
                return urllib.request.urlopen(req, timeout=60)
            except urllib.error.HTTPError as e:
                if e.code != 401 or self.token:
                    raise
                self._login(e.headers.get("WWW-Authenticate", ""))
        raise RuntimeError("login failed for " + url)

    def _login(self, challenge):
        fields = dict(re.findall(r'(\w+)="([^"]*)"', challenge))
        if "realm" not in fields:
            raise RuntimeError("the registry wants a login: " + challenge)
        query = {k: v for k, v in fields.items() if k in ("service", "scope")}
        with urllib.request.urlopen(fields["realm"] + "?" + urllib.parse.urlencode(query), timeout=60) as r:
            body = json.load(r)
        self.token = body.get("token") or body.get("access_token")

    def tags(self):
        out, path = [], "tags/list?n=1000"
        while path:
            with self._get(path) as r:
                out += json.load(r).get("tags") or []
                link = r.headers.get("Link", "")
            m = re.search(r'<([^>]+)>;\s*rel="next"', link)
            path = None
            if m:
                nxt = m.group(1)
                path = nxt.split("/tags/list", 1)[1] and "tags/list" + nxt.split("/tags/list", 1)[1]
        return out

    def digest(self, tag):
        with self._get("manifests/" + urllib.parse.quote(tag), "HEAD", {"Accept": ACCEPT}) as r:
            return r.headers["Docker-Content-Digest"]


def key(tag):
    m = VERSION.match(tag)
    return tuple(int(x) for x in m.group(1).split(".")) if m else None


def newest(registry, track):
    best = None
    for t in registry.tags():
        k = key(t)
        if k is None:
            continue
        v = ".".join(map(str, k))
        if track and not (v == track or v.startswith(track + ".")):
            continue
        # A bare "8" or "8.0" is a moving tag, not a release: a release has at least major.minor.
        if len(k) < 2 and not track:
            continue
        if best is None or k > best[0] or (k == best[0] and len(t) > len(best[1])):
            best = (k, t)
    return best[1] if best else None


def read(path):
    text = path.read_text()
    get = lambda name: (re.search(rf'^{name}\s*=\s*"([^"]*)"', text, re.M) or [None, ""])[1]
    return text, {n: get(n) for n in ("slug", "kind", "image", "track", "tag", "version", "digest")}


def put(text, name, value):
    return re.sub(rf'^({name}\s*=\s*)"[^"]*"', lambda m: f'{m.group(1)}"{value}"', text, count=1, flags=re.M)


def main(argv):
    write = "--write" in argv
    only = [a for a in argv if not a.startswith("-")]
    if not write and "--check" not in argv:
        print(__doc__)
        return 2
    failed = 0
    for path in sorted(HERE.glob("*.toml")):
        text, e = read(path)
        if e["kind"] != "container" or (only and e["slug"] not in only):
            continue
        host, repo = split_image(e["image"])
        try:
            reg = Registry(host, repo)
            tag = newest(reg, e["track"])
            if tag is None:
                raise RuntimeError(f"no release tag (track {e['track']!r})")
            digest = reg.digest(tag)
        except Exception as ex:  # one broken registry must not hide the others
            print(f"{e['slug']:18} ERROR {ex}")
            failed += 1
            continue
        version = ".".join(map(str, key(tag)))
        state = "same" if (tag, digest) == (e["tag"], e["digest"]) else ("NEWER" if tag != e["tag"] else "rebuilt")
        print(f"{e['slug']:18} {e['version'] or '-':>12} -> {version:<12} {state}")
        if write and state != "same":
            text = put(put(put(text, "version", version), "tag", tag), "digest", digest)
            path.write_text(text)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
