#!/usr/bin/env bash
# Consistency checks that keep a release publishable. Run by CI on every push,
# by `make release-check`, and (with --release) by the release workflow.
#
#   scripts/check-release-metadata.sh               # day to day
#   scripts/check-release-metadata.sh --release 0.1.0
#
# Checks: one workspace version used by every crate; repository/homepage URLs
# match the git remote; CHANGELOG has the right section; man pages lint and
# document every command-line flag; the Homebrew templates still point at real
# crates; Cargo.lock is up to date. Prints one line per check and exits
# non-zero if any failed.
set -uo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

release=""
if [ "${1:-}" = "--release" ]; then
    release="${2:?--release needs a version}"
    release="${release#v}"
fi

failures=0
ok()   { printf 'ok    %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }
check() { # check "description" command...
    local what="$1"; shift
    if "$@" >/dev/null 2>&1; then ok "$what"; else fail "$what"; fi
}

# --- version ---------------------------------------------------------------
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ok "workspace version $version" ;;
    *) fail "workspace version in Cargo.toml is not X.Y.Z ('$version')" ;;
esac
for crate in crates/*/Cargo.toml; do
    check "$crate uses the workspace version" grep -q '^version.workspace = true' "$crate"
done
if [ -n "$release" ]; then
    if [ "$release" = "$version" ]; then ok "tag version matches Cargo.toml"
    else fail "release $release but Cargo.toml says $version"; fi
fi

# --- URLs ------------------------------------------------------------------
slug="lab18net/bifold"
check "Cargo.toml repository is github.com/$slug" grep -q "^repository = \"https://github.com/$slug\"" Cargo.toml
check "Cargo.toml has no homepage (cargo warns it duplicates repository)" sh -c "! grep -q ^homepage Cargo.toml"
origin="$(git config --get remote.origin.url 2>/dev/null || true)"
case "$origin" in
    "") ok "no git remote to compare (skipped)" ;;
    *"$slug"*|*"$slug.git") ok "git remote origin is $slug" ;;
    *) fail "git remote origin is '$origin', expected $slug" ;;
esac
check "formula templates use the same repository" \
    sh -c "grep -q 'github.com/$slug' packaging/homebrew/bifold.rb.in && grep -q 'github.com/$slug' packaging/homebrew/bifold-gui.rb.in"

# --- changelog -------------------------------------------------------------
if [ -n "$release" ]; then
    check "CHANGELOG has a dated [$release] section" \
        sh -c "grep -Eq '^## \\[$release\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}\$' CHANGELOG.md && scripts/changelog-section.sh $release"
else
    check "CHANGELOG has an [Unreleased] or [$version] section" \
        sh -c "scripts/changelog-section.sh Unreleased || scripts/changelog-section.sh $version"
fi

# --- man pages -------------------------------------------------------------
for page in man/bifold.1 man/bifold-gui.1; do
    check "$page exists" test -f "$page"
done
if command -v mandoc >/dev/null 2>&1; then
    check "man pages lint clean (mandoc)" mandoc -Tlint -Wwarning man/bifold.1 man/bifold-gui.1
else
    ok "mandoc not installed, man lint skipped"
fi

# Every long option the programs print in --help must appear in their man page.
run_help() { # run_help crate
    if [ -x "target/debug/$1" ]; then "target/debug/$1" --help
    elif [ -x "target/release/$1" ]; then "target/release/$1" --help
    else cargo run -q -p "$2" -- --help
    fi
}
undocumented() { # undocumented binary package manpage
    local flags page missing=""
    flags="$(run_help "$1" "$2" 2>&1 | grep -o -- '--[a-z][a-z-]*' | sort -u)"
    page="$(sed 's/\\//g' "$3")"
    for flag in $flags; do
        printf '%s' "$page" | grep -q -- "$flag" || missing="$missing $flag"
    done
    printf '%s' "$missing"
}
for spec in "bifold bifold-tui man/bifold.1" "bifold-gui bifold-gui man/bifold-gui.1"; do
    set -- $spec
    missing="$(undocumented "$1" "$2" "$3")"
    if [ -z "$missing" ]; then ok "$3 documents every flag of $1"
    else fail "$3 does not document:$missing"; fi
done

# --- homebrew templates ----------------------------------------------------
for tpl in packaging/homebrew/bifold.rb.in packaging/homebrew/bifold-gui.rb.in; do
    check "$tpl has the @URL@/@SHA256@ placeholders" \
        sh -c "grep -q '@URL@' $tpl && grep -q '@SHA256@' $tpl"
done
check "bifold.rb.in builds crates/tui and installs its man page" \
    sh -c "grep -q 'path: \"crates/tui\"' packaging/homebrew/bifold.rb.in && grep -q 'man/bifold.1' packaging/homebrew/bifold.rb.in && test -f crates/tui/Cargo.toml"
check "bifold-gui.rb.in builds crates/gui and installs its man page" \
    sh -c "grep -q 'path: \"crates/gui\"' packaging/homebrew/bifold-gui.rb.in && grep -q 'man/bifold-gui.1' packaging/homebrew/bifold-gui.rb.in && test -f crates/gui/Cargo.toml"

check "cask template has @VERSION@/@URL@/@SHA256@ and installs Bifold.app" \
    sh -c "f=packaging/homebrew/cask/bifold-app.rb.in; grep -q @VERSION@ \$f && grep -q @URL@ \$f && grep -q @SHA256@ \$f && grep -q 'app \"Bifold.app\"' \$f && grep -q 'github.com/$slug' \$f"

# --- macOS app bundle --------------------------------------------------------
check "packaging/macos/Bifold.icns exists" test -s packaging/macos/Bifold.icns
check "Info.plist template names the bifold-gui executable and the icon" \
    sh -c "grep -q '<string>bifold-gui</string>' packaging/macos/Info.plist.in && grep -q '<string>Bifold</string>' packaging/macos/Info.plist.in && grep -q '@VERSION@' packaging/macos/Info.plist.in"
check "scripts/package-macos.sh is executable" test -x scripts/package-macos.sh

# --- lockfile --------------------------------------------------------------
check "Cargo.lock is up to date (--locked)" cargo metadata --locked --format-version 1

if [ "$failures" -gt 0 ]; then
    printf '\n%d check(s) failed\n' "$failures" >&2
    exit 1
fi
printf '\nall checks passed\n'
