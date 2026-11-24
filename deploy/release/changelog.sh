#!/usr/bin/env bash
# The changelog of a release (T19.11): the commit subjects since the previous tag, grouped by
# their conventional-commit type.
#
#   deploy/release/changelog.sh v1.0.0 [v0.9.0]     (the previous tag defaults to the one before)
set -euo pipefail
tag="${1:?usage: changelog.sh TAG [PREVIOUS_TAG]}"
prev="${2:-$(git describe --tags --abbrev=0 "$tag^" 2> /dev/null || true)}"
range="${prev:+$prev..}$tag"
echo "# Built Panel ${tag#v}"
echo
echo "Changes since ${prev:-the beginning}:"
for entry in "feat:New" "fix:Fixed" "perf:Faster" "security:Security" "test:Tests" "docs:Documentation" "refactor:Internal" "chore:Maintenance" "build:Build" "ci:Build"; do
    type="${entry%%:*}"; title="${entry#*:}"
    lines=$(git log --no-merges --format='%s' "$range" | grep -E "^${type}(\([^)]*\))?!?: " | sed -E "s/^${type}(\(([^)]*)\))?!?: /\2|/" || true)
    [ -n "$lines" ] || continue
    echo; echo "## $title"
    # "scope|text" becomes "- **scope**: text", a missing scope just "- text".
    echo "$lines" | sed -E 's/^([^|]+)\|(.*)$/- **\1**: \2/; s/^\|(.*)$/- \1/'
done
breaking=$(git log --no-merges --format='%s' "$range" | grep -E '^[a-z]+(\([^)]*\))?!: ' || true)
[ -z "$breaking" ] || { echo; echo "## Breaking changes"; echo "$breaking" | sed 's/^/- /'; }
