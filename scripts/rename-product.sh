#!/usr/bin/env bash
# Rename the product across the repo: display name, binary/config slug, env prefix.
#
#   scripts/rename-product.sh Shunt [--dry-run]
#
# Only package names and strings change; directory names (crates/core, …) stay
# put, so the workspace keeps working. Cargo.lock is left for cargo to refresh.
# Man pages and Homebrew formula templates are renamed too. The GitHub repo, the
# Homebrew tap and the release tags are NOT touched: rename those on GitHub.
set -euo pipefail

if [[ $# -lt 1 || "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  echo "usage: scripts/rename-product.sh <NewName> [--dry-run]"
  echo "  e.g. scripts/rename-product.sh Shunt"
  exit "${1:+0}"
fi

DISPLAY="$1"
DRY_RUN=0
[[ "${2:-}" == "--dry-run" ]] && DRY_RUN=1

SLUG=$(printf '%s' "$DISPLAY" | tr '[:upper:]' '[:lower:]' | tr -cs '[:alnum:]' '-' | sed 's/^-//; s/-$//')
UPPER=$(printf '%s' "$SLUG" | tr -d '-' | tr '[:lower:]' '[:upper:]')

if [[ -z "$SLUG" || ! "$SLUG" =~ ^[a-z][a-z0-9-]*$ ]]; then
  echo "error: '$DISPLAY' yields an unusable slug ('$SLUG'); use letters and digits" >&2
  exit 1
fi

OLD_DISPLAY="Bifold"
OLD_SLUG="bifold"
OLD_UPPER="BIFOLD"

if [[ "$SLUG" == "$OLD_SLUG" ]]; then
  echo "nothing to do: already named '$OLD_DISPLAY'"
  exit 0
fi

cd "$(git rev-parse --show-toplevel 2>/dev/null || echo .)"

LIST=$(git ls-files -co --exclude-standard \
  | grep -Ev '^(target/|Cargo.lock$)' \
  | grep -E '\.(rs|md|toml|sh|ya?ml|1)$|\.rb\.in$|(^|/)Makefile$' \
  | grep -v '^docs/naming.md$' || true)   # keeps the naming history intact

if [[ -z "$LIST" ]]; then
  echo "error: no source files found (run from the repo)" >&2
  exit 1
fi

FILE_COUNT=$(printf '%s\n' "$LIST" | wc -l | tr -d ' ')

echo "renaming:"
echo "  display : $OLD_DISPLAY -> $DISPLAY"
echo "  slug    : $OLD_SLUG -> $SLUG        (binaries, crates, ~/.config/$SLUG)"
echo "  env     : ${OLD_UPPER}_ -> ${UPPER}_"

TOTAL=0
while IFS= read -r file; do
  [[ -f "$file" ]] || continue
  hits=$(perl -ne 'BEGIN{$n=0} $n += () = /\Q'"$OLD_DISPLAY"'\E|\Q'"$OLD_SLUG"'\E|\Q'"$OLD_UPPER"'\E/g; END{print "$n\n"}' "$file")
  if [[ "$hits" -gt 0 ]]; then
    printf '  %-42s %s hit(s)\n' "$file" "$hits"
    TOTAL=$((TOTAL + hits))
    if [[ $DRY_RUN -eq 0 ]]; then
      perl -pi -e "s/\Q$OLD_UPPER\E/$UPPER/g; s/\Q$OLD_DISPLAY\E/$DISPLAY/g; s/\Q$OLD_SLUG\E/$SLUG/g" "$file"
    fi
  fi
done <<< "$LIST"

# Files named after the product: man pages and Homebrew formula templates.
for old_file in man/"$OLD_SLUG"*.1 packaging/homebrew/"$OLD_SLUG"*.rb.in packaging/homebrew/cask/"$OLD_SLUG"*.rb.in; do
  [[ -f "$old_file" ]] || continue
  new_file="$(dirname "$old_file")/${SLUG}$(basename "$old_file" | sed "s/^$OLD_SLUG//")"
  printf '  %-42s -> %s\n' "$old_file" "$new_file"
  if [[ $DRY_RUN -eq 0 ]]; then
    if git ls-files --error-unmatch "$old_file" >/dev/null 2>&1; then git mv "$old_file" "$new_file"; else mv "$old_file" "$new_file"; fi
  fi
done

echo
echo "$TOTAL occurrence(s) in $FILE_COUNT file(s)$([[ $DRY_RUN -eq 1 ]] && echo ' — dry run, nothing written')"
if [[ $DRY_RUN -eq 0 ]]; then
  cat <<EOF

next:
  cargo build --workspace && cargo test --workspace
  git diff --stat
  # optional: rename the checkout itself, e.g. mv "$(pwd)" "../$DISPLAY"
EOF
fi
