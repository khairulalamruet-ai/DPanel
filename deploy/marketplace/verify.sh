#!/usr/bin/env bash
# Runs every container entry of the marketplace the way the panel runs a client container (no
# capabilities, no new privileges, 256 processes, its memory limit, a data volume, a temporary
# /tmp) and checks that it is still running after 40 seconds. Needs Docker; run it on the build
# box after refresh.py moved a version. Usage: verify.sh [slug ...]
set -uo pipefail
cd "$(dirname "$0")"
bad=0
for f in *.toml; do
  grep -q '^kind *= *"container"' "$f" || continue
  slug=${f%.toml}
  [ $# -eq 0 ] || [[ " $* " == *" $slug "* ]] || continue
  val() { grep -m1 "^$1 *=" "$f" | sed 's/^[^=]*= *"\{0,1\}//; s/"\{0,1\}$//'; }
  ref="$(val image)@$(val digest)"; mem=$(grep -m1 '^memory_mb' "$f" | sed 's/[^0-9]//g')
  envs=(); in_env=0
  while IFS= read -r line; do
    [ "$line" = "[env]" ] && { in_env=1; continue; }
    [ $in_env = 1 ] && [[ "$line" == *=* ]] && envs+=(-e "$(echo "${line%%=*}" | tr -d ' ')=$(echo "${line#*= }" | sed 's/^"//; s/"$//')")
  done < "$f"
  n="mpv-$slug"; docker rm -f "$n" > /dev/null 2>&1; docker volume rm -f "v-$n" > /dev/null 2>&1; docker volume create "v-$n" > /dev/null
  docker run -d --name "$n" --cap-drop ALL --security-opt no-new-privileges --pids-limit 256 -m "${mem}m" --tmpfs /tmp -v "v-$n:/data" ${envs[@]+"${envs[@]}"} "$ref" > /dev/null 2>"/tmp/$n.err" || { echo "FAIL  $slug: $(tail -1 /tmp/$n.err)"; bad=1; continue; }
  sleep 40
  st=$(docker inspect -f '{{.State.Status}}' "$n" 2>/dev/null)
  if [ "$st" = running ]; then echo "ok    $slug ($ref)"; else echo "FAIL  $slug: $st"; docker logs --tail 3 "$n" 2>&1 | cut -c1-150 | sed 's/^/        /'; bad=1; fi
  docker rm -f "$n" > /dev/null 2>&1; docker volume rm -f "v-$n" > /dev/null 2>&1; docker rmi -f "$ref" > /dev/null 2>&1
done
exit $bad
