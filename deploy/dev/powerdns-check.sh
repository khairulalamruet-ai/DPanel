#!/usr/bin/env bash
# Checks that the dev PowerDNS (deploy/dev/compose.yml) works through its API
# the way the panel will use it: create a zone, read it, change a record, ask
# the server itself for the answer over DNS, delete the zone (T05.07).
#
#   PDNS_API_KEY=... deploy/dev/powerdns-check.sh [api-url] [dns-port]
#
# The key is read from the environment and never printed.
set -euo pipefail
: "${PDNS_API_KEY:?set PDNS_API_KEY}"
API="${1:-http://127.0.0.1:8053}/api/v1/servers/localhost"
DNS_PORT="${2:-5300}"
ZONE="builtpanel-check-$$.test."
call() { # method path [json]
  curl -sS -X "$1" -H "X-API-Key: $PDNS_API_KEY" -H 'Content-Type: application/json' \
    ${3:+-d "$3"} -w '\n%{http_code}' "$API$2"
}
expect() { # expected-status description actual-output
  local status; status="$(printf '%s' "$3" | tail -n1)"
  if [ "$status" != "$1" ]; then echo "FAILED: $2 (HTTP $status)"; printf '%s\n' "$3" | head -n -1 | head -c 400; echo; exit 1; fi
  echo "ok: $2"
}
trap 'call DELETE "/zones/$ZONE" >/dev/null 2>&1 || true' EXIT

out="$(call GET "")"; expect 200 "the API answers" "$out"
out="$(call POST /zones "{\"name\":\"$ZONE\",\"kind\":\"Native\",\"nameservers\":[\"ns1.$ZONE\"]}")"
expect 201 "a zone is created" "$out"
out="$(call GET "/zones/$ZONE")"; expect 200 "the zone can be read" "$out"
printf '%s' "$out" | grep -q '"type": "SOA"' && echo "ok: it has an SOA record"
out="$(call PATCH "/zones/$ZONE" "{\"rrsets\":[{\"name\":\"www.$ZONE\",\"type\":\"A\",\"ttl\":300,\"changetype\":\"REPLACE\",\"records\":[{\"content\":\"192.0.2.10\",\"disabled\":false}]}]}")"
expect 204 "a record is added" "$out"
if command -v dig >/dev/null 2>&1; then
  answer="$(dig +short +time=3 +tries=1 -p "$DNS_PORT" @127.0.0.1 "www.$ZONE" A)"
  [ "$answer" = "192.0.2.10" ] && echo "ok: the server answers www.$ZONE with $answer over DNS" || { echo "FAILED: DNS answer was '$answer'"; exit 1; }
fi
out="$(call DELETE "/zones/$ZONE")"; expect 204 "the zone is deleted" "$out"
trap - EXIT
echo "PowerDNS works."
