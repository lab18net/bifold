#!/usr/bin/env bash
# Cut a release: bump the version, date the changelog, run every check, commit,
# tag. Nothing is pushed unless you say so.
#
#   scripts/release.sh 0.1.0              # prepare commit + tag locally
#   scripts/release.sh 0.1.0 --dry-run    # check and show, change nothing
#   scripts/release.sh 0.1.0 --push       # ...and push main and the tag
#
# Pushing the tag starts .github/workflows/release.yml, which publishes the
# GitHub release; the Homebrew tap then picks it up (see packaging/homebrew/).
# SKIP_CHECKS=1 skips `make check` (only for testing this script).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

slug="lab18net/bifold"
version="" push=0 dry=0
while [ $# -gt 0 ]; do
    case "$1" in
        --push) push=1 ;;
        --dry-run) dry=1 ;;
        -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *) [ -z "$version" ] || { echo "one version only" >&2; exit 2; }; version="${1#v}" ;;
    esac
    shift
done
[ -n "$version" ] || { echo "usage: scripts/release.sh VERSION [--push] [--dry-run]" >&2; exit 2; }
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
    echo "version must look like 1.2.3 or 1.2.3-rc.1, got '$version'" >&2; exit 2
fi
tag="v$version"
date="$(date -u +%Y-%m-%d)"

step() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
do_it() { if [ "$dry" -eq 1 ]; then printf '    [dry-run] %s\n' "$*"; else "$@"; fi; }
command -v python3 >/dev/null || die "python3 is needed to edit CHANGELOG.md"

# --- preflight (read-only) -------------------------------------------------
step "preflight"
[ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || die "release from the main branch"
[ -z "$(git status --porcelain)" ] || die "the working tree is not clean; commit or stash first"
if git config --get remote.origin.url >/dev/null; then
    git fetch --quiet origin main --tags
    local_sha="$(git rev-parse HEAD)"
    remote_sha="$(git rev-parse origin/main)"
    if [ "$local_sha" != "$remote_sha" ]; then
        git merge-base --is-ancestor "$remote_sha" "$local_sha" \
            || die "origin/main has commits you do not have; pull first"
        echo "    note: main is ahead of origin/main; --push will publish those commits too"
    fi
    [ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ] || die "tag $tag already exists on origin"
fi
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists"
grep -q "^## \[$version\]" CHANGELOG.md && die "CHANGELOG.md already has a [$version] section"
scripts/changelog-section.sh Unreleased >/dev/null || die "write the release notes under [Unreleased] in CHANGELOG.md first"
prev="$(git tag --list 'v[0-9]*' --sort=-v:refname | head -1 || true)"
echo "    version $version (tag $tag, previous: ${prev:-none}), date $date"

# --- changes ---------------------------------------------------------------
current="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
if [ "$current" != "$version" ]; then
    step "set workspace version $current -> $version"
    do_it sed -i.bak "s/^version = \"$current\"\$/version = \"$version\"/" Cargo.toml
    do_it rm -f Cargo.toml.bak
    do_it cargo update --workspace --offline --quiet
else
    step "workspace version is already $version"
fi

step "date the changelog and the man pages"
if [ "$dry" -eq 1 ]; then
    echo "    [dry-run] CHANGELOG.md: [Unreleased] -> new empty [Unreleased] + [$version] - $date"
    echo "    [dry-run] man/*.1: .TH date -> $date"
else
    VERSION="$version" DATE="$date" PREV="$prev" SLUG="$slug" python3 - <<'PY'
import os, re, pathlib
v, d, prev, slug = (os.environ[k] for k in ("VERSION", "DATE", "PREV", "SLUG"))
p = pathlib.Path("CHANGELOG.md")
t = p.read_text()
t = t.replace("## [Unreleased]\n", f"## [Unreleased]\n\n## [{v}] - {d}\n", 1)
base = f"https://github.com/{slug}"
new_ref = (f"[{v}]: {base}/compare/{prev}...v{v}" if prev else f"[{v}]: {base}/releases/tag/v{v}")
t = re.sub(r"^\[Unreleased\]: .*$", f"[Unreleased]: {base}/compare/v{v}...HEAD\n{new_ref}", t, flags=re.M)
p.write_text(t)
for page in pathlib.Path("man").glob("*.1"):
    s = page.read_text()
    s = re.sub(r'^(\.TH \S+ 1 )"[^"]*"', rf'\1"{d}"', s, count=1, flags=re.M)
    page.write_text(s)
PY
fi

# --- verify ----------------------------------------------------------------
if [ "$dry" -eq 1 ]; then
    step "checks (dry run: not executed)"
    echo "    [dry-run] make check; scripts/check-release-metadata.sh --release $version"
else
    if [ "${SKIP_CHECKS:-0}" = "1" ]; then
        step "make check SKIPPED (SKIP_CHECKS=1)"
    else
        step "make check"
        make check
    fi
    step "release metadata"
    scripts/check-release-metadata.sh --release "$version"
fi

# --- commit and tag ----------------------------------------------------------
step "commit and tag $tag"
do_it git add Cargo.toml Cargo.lock CHANGELOG.md man/bifold.1 man/bifold-gui.1
do_it git commit --quiet -m "Release $tag"
if [ "$dry" -eq 1 ]; then
    echo "    [dry-run] git tag -a $tag (message: the [$version] release notes)"
else
    printf 'Bifold %s\n\n%s\n' "$version" "$(scripts/changelog-section.sh "$version")" | git tag -a "$tag" -F -
fi

if [ "$dry" -eq 1 ]; then
    printf '\nDry run: nothing was changed.\n'
elif [ "$push" -eq 1 ]; then
    step "push main and $tag"
    do_it git push origin main
    do_it git push origin "$tag"
    echo "Pushed. Watch the release workflow: https://github.com/$slug/actions"
else
    cat <<EOF

Done locally (nothing pushed). Review with:  git show $tag --stat
To publish:                                   git push origin main && git push origin $tag
(or rerun with --push)
EOF
fi
