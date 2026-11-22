#!/usr/bin/env bash
# Build dist/Bifold.app (the desktop UI with its icon) and a zip of it.
#
#   scripts/package-macos.sh              # build release binary, bundle, sign ad hoc, zip
#   scripts/package-macos.sh --no-build   # reuse target/release/bifold-gui
#   scripts/package-macos.sh --out DIR    # default: dist
#
# The bundle is signed ad hoc (`codesign -s -`) unless CODESIGN_IDENTITY names a
# Developer ID; it is not notarised, so a downloaded copy needs right-click ->
# Open the first time (see README). The icon is packaging/macos/Bifold.icns.
set -euo pipefail
[ "$(uname)" = "Darwin" ] || { echo "macOS only" >&2; exit 1; }

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
build=1 out="dist"
while [ $# -gt 0 ]; do
    case "$1" in
        --no-build) build=0 ;;
        --out) out="${2:?--out needs a directory}"; shift ;;
        -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
[ "$build" -eq 0 ] || cargo build --release --locked -p bifold-gui
bin="target/release/bifold-gui"
[ -x "$bin" ] || { echo "missing $bin (build first)" >&2; exit 1; }

arch="$(uname -m)"; [ "$arch" = "arm64" ] && arch="aarch64"
app="$out/Bifold.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/bifold-gui"
cp packaging/macos/Bifold.icns "$app/Contents/Resources/Bifold.icns"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist.in > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
plutil -lint "$app/Contents/Info.plist" >/dev/null

codesign --force --deep --sign "${CODESIGN_IDENTITY:--}" "$app"
codesign --verify --deep --strict "$app"

zip="$out/Bifold-$version-$arch.app.zip"
rm -f "$zip" "$zip.sha256"
ditto -c -k --keepParent "$app" "$zip"
(cd "$out" && shasum -a 256 "$(basename "$zip")" > "$(basename "$zip").sha256")
echo "built $app"
echo "built $zip"
