#!/usr/bin/env bash
# Coraza WAF in the Caddy image (T11.01), for real, on the build box: builds deploy/caddy, runs it
# in blocking and in detecting mode and sends it ordinary, SQL injection and XSS requests.
# `deploy/dev/remote.sh sh "bash deploy/dev/waf-live.sh"`
set -uo pipefail
cd "$(dirname "$0")/../caddy"
docker build -q -t bp-caddy-waf:test . > /dev/null || { echo "build FAILED"; exit 1; }
fail=0
check() { # name expected actual
    if [ "$2" = "$3" ]; then echo "ok   $1 -> $3"; else echo "FAIL $1: wanted $2, got $3"; fail=1; fi
}
run() { # mode port
    docker rm -f "bp-waf-$1" > /dev/null 2>&1
    docker run -d --name "bp-waf-$1" -p "$2:8080" -e "WAF_MODE=$3" \
        -v "$PWD/waf-test.Caddyfile:/etc/caddy/Caddyfile:ro" bp-caddy-waf:test > /dev/null
    for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$2/" && break; sleep 0.5; done
}
code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
run block 18081 On
check "ordinary page" 200 "$(code http://127.0.0.1:18081/)"
check "page with a normal query" 200 "$(code 'http://127.0.0.1:18081/?page=2&name=alice')"
check "SQL injection (query)" 403 "$(code --get --data-urlencode "id=1' OR '1'='1" http://127.0.0.1:18081/)"
check "SQL injection (union)" 403 "$(code --get --data-urlencode "id=1 UNION SELECT username,password FROM users" http://127.0.0.1:18081/)"
check "XSS (query)" 403 "$(code --get --data-urlencode "q=<script>alert(1)</script>" http://127.0.0.1:18081/)"
check "XSS (post body)" 403 "$(code -X POST --data-urlencode "comment=<img src=x onerror=alert(1)>" http://127.0.0.1:18081/)"
check "path traversal" 403 "$(code --path-as-is 'http://127.0.0.1:18081/../../etc/passwd')"
check "an ordinary post" 200 "$(code -X POST --data-urlencode 'comment=hello there' http://127.0.0.1:18081/)"
run detect 18082 DetectionOnly
check "detection only lets SQL injection through" 200 "$(code --get --data-urlencode "id=1' OR '1'='1" http://127.0.0.1:18082/)"
docker rm -f bp-waf-block bp-waf-detect > /dev/null 2>&1
[ "$fail" = 0 ] && echo "WAF: all checks passed" || { echo "WAF: FAILED"; exit 1; }
