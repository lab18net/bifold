#!/usr/bin/env bash
# Render the Homebrew formulae for one Bifold release.
#
#   scripts/update-formula.sh 0.1.0 --tap ../homebrew-tap
#   scripts/update-formula.sh 0.1.0 --tarball /tmp/bifold-0.1.0.tar.gz --tap /tmp/tap
#   ... --app-zip dist/Bifold-0.1.0-aarch64.app.zip   # also try the cask locally
#
# Reads packaging/homebrew/*.rb.in, fills in the source URL and its SHA-256 and
# writes <tap>/Formula/bifold.rb and <tap>/Formula/bifold-gui.rb. The tarball is
# GitHub's archive of tag v<version> (it must be pushed), or --tarball FILE for
# a local one (then the URL is file://FILE unless --url is given), which is how
# a formula is tried before anything is published.
#
# It also writes <tap>/Casks/bifold-app.rb from packaging/homebrew/cask/. Its
# SHA-256 is that of the release asset Bifold-<version>-aarch64.app.zip
# (downloaded), or of --app-zip FILE (then the cask points at file://FILE).
set -euo pipefail

REPO_SLUG="lab18net/bifold"
root="$(cd "$(dirname "$0")/.." && pwd)"

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit "${1:-2}"
}

version="" tap="" tarball="" url="" appzip=""
while [ $# -gt 0 ]; do
    case "$1" in
        --tap) tap="${2:?--tap needs a directory}"; shift 2 ;;
        --tarball) tarball="${2:?--tarball needs a file}"; shift 2 ;;
        --app-zip) appzip="${2:?--app-zip needs a file}"; shift 2 ;;
        --url) url="${2:?--url needs a value}"; shift 2 ;;
        -h|--help) usage 0 ;;
        -*) echo "unknown option: $1" >&2; usage ;;
        *) [ -z "$version" ] || usage; version="$1"; shift ;;
    esac
done
[ -n "$version" ] && [ -n "$tap" ] || usage
version="${version#v}"
case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "version must look like 1.2.3, got '$version'" >&2; exit 2 ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [ -n "$tarball" ]; then
    [ -f "$tarball" ] || { echo "no such file: $tarball" >&2; exit 1; }
    file="$tarball"
    [ -n "$url" ] || url="file://$(cd "$(dirname "$tarball")" && pwd)/$(basename "$tarball")"
else
    [ -n "$url" ] || url="https://github.com/$REPO_SLUG/archive/refs/tags/v$version.tar.gz"
    file="$tmp/source.tar.gz"
    if ! curl -fsSL --retry 3 "$url" -o "$file"; then
        echo "cannot download $url" >&2
        echo "Has the tag v$version been pushed? (git push origin v$version)" >&2
        exit 1
    fi
fi

sha="$(shasum -a 256 "$file" | cut -d' ' -f1)"
[ "${#sha}" -eq 64 ] || { echo "could not compute a SHA-256" >&2; exit 1; }

mkdir -p "$tap/Formula"
for template in "$root"/packaging/homebrew/*.rb.in; do
    name="$(basename "$template" .rb.in)"
    sed -e "s|@URL@|$url|" -e "s|@SHA256@|$sha|" "$template" > "$tap/Formula/$name.rb"
    echo "wrote $tap/Formula/$name.rb"
done
echo "url    $url"
echo "sha256 $sha"

# --- cask: the prebuilt, ad-hoc signed Bifold.app from the GitHub release ----
cask_url='https://github.com/'"$REPO_SLUG"'/releases/download/v#{version}/Bifold-#{version}-aarch64.app.zip'
if [ -n "$appzip" ]; then
    [ -f "$appzip" ] || { echo "no such file: $appzip" >&2; exit 1; }
    zipfile="$appzip"
    cask_url="file://$(cd "$(dirname "$appzip")" && pwd)/$(basename "$appzip")"
else
    real="https://github.com/$REPO_SLUG/releases/download/v$version/Bifold-$version-aarch64.app.zip"
    zipfile="$tmp/app.zip"
    if ! curl -fsSL --retry 3 "$real" -o "$zipfile"; then
        echo "cannot download $real (is the release published?); cask not written" >&2
        exit 1
    fi
fi
zsha="$(shasum -a 256 "$zipfile" | cut -d' ' -f1)"
mkdir -p "$tap/Casks"
sed -e "s|@VERSION@|$version|" -e "s|@URL@|$cask_url|" -e "s|@SHA256@|$zsha|" \
    "$root/packaging/homebrew/cask/bifold-app.rb.in" > "$tap/Casks/bifold-app.rb"
echo "wrote $tap/Casks/bifold-app.rb"
echo "cask sha256 $zsha"
