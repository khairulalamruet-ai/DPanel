#!/usr/bin/env bash
# deploy/release/server-deploy.sh for real (T19.11 hardening), on the build box as root:
#   deploy/dev/remote.sh sh "sudo -n bash deploy/dev/deploy-live.sh"
# The deploy script itself is replaced by one that only notes it was called, so nothing on the
# box changes; what is tested is what server-deploy.sh lets through.
set -uo pipefail
cd "$(dirname "$0")/../.."
BPCTL="${BUILTPANEL_BPCTL:-/home/dev/target-rust/debug/bpctl}"
[ -x "$BPCTL" ] || { echo "no bpctl at $BPCTL"; exit 2; }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
STATE="$T/state"; mkdir -p "$STATE/incoming"
CALLED="$T/called"
cat > "$T/fake-deploy.sh" <<FAKE
#!/usr/bin/env bash
echo "\$@" > "$CALLED"; ls "\${2}" >> "$CALLED"; if [ "\${3:-}" = --web-dir ]; then ls "\${4}" >> "$CALLED"; fi
FAKE
chmod +x "$T/fake-deploy.sh"
"$BPCTL" release keygen --kid good-1 --out "$T/good.key" > "$T/keys.txt"
"$BPCTL" release keygen --kid evil-1 --out "$T/evil.key" > /dev/null
export BUILTPANEL_DEPLOY_STATE="$STATE" BUILTPANEL_RELEASE_KEYS="$T/keys.txt" BUILTPANEL_DEPLOY_SCRIPT="$T/fake-deploy.sh" BUILTPANEL_BPCTL="$BPCTL"
fail=0
NAME=builtpanel-1.2.3-linux-x86_64

# A release: the archive, and the manifest signed over its programs.
build() { # label key kid [files-in-manifest]  (the web archives are in the release when WITH_WEB=1)
    rm -rf "$T/rel" "$STATE/incoming"/*; mkdir -p "$T/rel/$NAME/bin"
    printf 'api' > "$T/rel/$NAME/bin/builtpanel-control-api"; printf 'ctl' > "$T/rel/$NAME/bin/bpctl"
    chmod +x "$T/rel/$NAME/bin"/*
    local files="builtpanel-control-api bpctl"
    if [ "${WITH_WEB:-0}" = 1 ]; then
        mkdir -p "$T/rel/$NAME/web" "$T/app"; printf '<html>client</html>' > "$T/app/index.html"
        tar -czf "$T/rel/$NAME/web/web-client" -C "$T/app" .; printf '<html>admin</html>' > "$T/app/index.html"
        tar -czf "$T/rel/$NAME/web/web-admin" -C "$T/app" .
        files="$files web-client web-admin"
    fi
    ( cd "$T/rel/$NAME" && cp web/* bin/ 2>/dev/null; cd bin && "$BPCTL" release sign --key "$2" --kid "$3" --version 1.2.3 --commit abc123 --out "$T/manifest.json" ${4:-$files} > /dev/null; rm -f web-client web-admin )
    tar -czf "$STATE/incoming/$NAME.tar.gz" -C "$T/rel" "$NAME"
    cp "$T/manifest.json" "$STATE/incoming/release-manifest.json"
    rm -f "$CALLED"
}
run() { "$PWD/deploy/release/server-deploy.sh" "$NAME" > "$T/out" 2>&1; }
expect() { # label, want (pass|refuse)
    local called=no; [ -f "$CALLED" ] && called=yes
    if { [ "$2" = pass ] && [ "$called" = yes ] && [ "$rc" = 0 ]; } || { [ "$2" = refuse ] && [ "$called" = no ] && [ "$rc" != 0 ]; }; then
        echo "ok    $1"
    else echo "FAIL  $1 (exit $rc, deploy called: $called)"; sed 's/^/        /' "$T/out" | tail -4; fail=1; fi
}

build good "$T/good.key" good-1; run; rc=$?; expect "a release signed by a trusted key is deployed" pass
grep -q "bpctl" "$CALLED" && echo "ok    the deploy script got the checked programs" || { echo "FAIL  no programs handed over"; fail=1; }
[ -z "$(ls "$STATE" | grep '^work\.')" ] && echo "ok    the working folder is gone" || { echo "FAIL  working folder left"; fail=1; }
[ -z "$(ls "$STATE/incoming")" ] && echo "ok    the incoming folder is emptied" || { echo "FAIL  incoming not emptied"; fail=1; }

build evil "$T/evil.key" evil-1; run; rc=$?; expect "a manifest signed by a key that is not trusted is refused" refuse
build lying "$T/evil.key" good-1; run; rc=$?; expect "a manifest that claims a trusted key id with another key is refused" refuse

build tamper "$T/good.key" good-1; mkdir "$T/ex"; tar -xzf "$STATE/incoming/$NAME.tar.gz" -C "$T/ex"; printf 'EVIL' > "$T/ex/$NAME/bin/builtpanel-control-api"; tar -czf "$STATE/incoming/$NAME.tar.gz" -C "$T/ex" "$NAME"; rm -rf "$T/ex"; run; rc=$?; expect "a program changed after signing is refused" refuse

build missing "$T/good.key" good-1 builtpanel-control-api; run; rc=$?; expect "a signed manifest that does not list bpctl is refused" refuse

build link "$T/good.key" good-1; mkdir "$T/ex"; tar -xzf "$STATE/incoming/$NAME.tar.gz" -C "$T/ex"; ln -s /etc/passwd "$T/ex/$NAME/bin/extra"; tar -czf "$STATE/incoming/$NAME.tar.gz" -C "$T/ex" "$NAME"; rm -rf "$T/ex"; run; rc=$?; expect "a link inside bin is refused" refuse

build escape "$T/good.key" good-1; mkdir "$T/ex"; echo x > "$T/ex/x"; tar -czf "$STATE/incoming/$NAME.tar.gz" -C "$T/ex" --transform "s|^x|../../escape|" x 2>/dev/null; rm -rf "$T/ex"; run; rc=$?; expect "an archive with a path that climbs out is refused" refuse
[ ! -e "$T/../escape" ] && echo "ok    nothing was written outside" || { echo "FAIL  something escaped"; fail=1; }

# The web interface travels in the release and is signed with it.
WITH_WEB=1 build web "$T/good.key" good-1; run; rc=$?; expect "a release with its web archives is deployed, the archives handed over" pass
grep -q "web-client" "$CALLED" && grep -q "web-admin" "$CALLED" && echo "ok    the web archives reached the deploy script" || { echo "FAIL  no web archives handed over"; fail=1; }
WITH_WEB=1 build webtamper "$T/good.key" good-1; mkdir "$T/ex"; tar -xzf "$STATE/incoming/$NAME.tar.gz" -C "$T/ex"; printf 'EVIL' > "$T/ex/$NAME/web/web-admin"; tar -czf "$STATE/incoming/$NAME.tar.gz" -C "$T/ex" "$NAME"; rm -rf "$T/ex"; run; rc=$?; expect "a web archive changed after signing is refused" refuse
WITH_WEB=1 build webmissing "$T/good.key" good-1 "builtpanel-control-api bpctl"; run; rc=$?; expect "web archives that the signed manifest does not list are refused" refuse

build plain "$T/good.key" good-1; rm "$STATE/incoming/release-manifest.json"; run; rc=$?; expect "no manifest, no deploy" refuse
build name "$T/good.key" good-1; rc=0; "$PWD/deploy/release/server-deploy.sh" "../../etc/passwd" > "$T/out" 2>&1 && rc=0 || rc=$?; [ "$rc" != 0 ] && [ ! -f "$CALLED" ] && echo "ok    a name that is not a release name is refused" || { echo "FAIL  odd name accepted"; fail=1; }
exit $fail
