#!/usr/bin/env bash
# Print the CHANGELOG.md section of one version (without its heading).
#   scripts/changelog-section.sh 0.1.0
#   scripts/changelog-section.sh Unreleased
# Exits 1 when the section is missing or empty.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
want="${1:?usage: changelog-section.sh VERSION|Unreleased}"
want="${want#v}"

body="$(awk -v want="$want" '
    /^## \[/ {
        name = $0
        sub(/^## \[/, "", name); sub(/\].*$/, "", name)
        printing = (name == want)
        next
    }
    /^\[[^]]+\]: / { printing = 0 }
    printing { print }
' "$root/CHANGELOG.md")"

# Trim leading and trailing blank lines.
body="$(printf '%s\n' "$body" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}')"
if [ -z "$(printf '%s' "$body" | tr -d '[:space:]')" ]; then
    echo "CHANGELOG.md has no (or an empty) section for [$want]" >&2
    exit 1
fi
printf '%s\n' "$body"
