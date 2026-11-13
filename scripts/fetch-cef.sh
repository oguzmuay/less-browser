#!/usr/bin/env bash
# Downloads the CEF "minimal" distribution into .cef/ in the layout cef-dll-sys expects (CEF_PATH).
# Needed because download-cef 3.0.0 fails on the CDN index (index.json > ureq's 10 MB body limit).
# CEF_VERSION must match the build metadata of the pinned `cef` crate (cef = "=156.4.0+156.0.26").
set -euo pipefail

CEF_VERSION="156.0.26"
CDN="${CEF_DOWNLOAD_URL:-https://cef-builds.spotifycdn.com}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/.cef"
case "$(uname -m)" in
  arm64) PLATFORM=macosarm64 ;;
  x86_64) PLATFORM=macosx64 ;;
  *) echo "unsupported arch" >&2; exit 1 ;;
esac

if [[ -f "$DEST/archive.json" ]] && grep -q "cef_binary_${CEF_VERSION}+" "$DEST/archive.json"; then
  echo "CEF $CEF_VERSION already in $DEST"; exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
curl -fsSL "$CDN/index.json" -o "$WORK/index.json"
read -r NAME SHA1 < <(python3 -I -c '
import json, sys
idx = json.load(open(sys.argv[1]))
v = next(v for v in idx[sys.argv[2]]["versions"] if v["cef_version"].startswith(sys.argv[3] + "+"))
f = next(f for f in v["files"] if f["type"] == "minimal")
print(f["name"], f["sha1"])' "$WORK/index.json" "$PLATFORM" "$CEF_VERSION")

echo "Downloading $NAME"
curl -fL --progress-bar "$CDN/$NAME" -o "$WORK/$NAME"
echo "$SHA1  $WORK/$NAME" | shasum -a 1 -c -
mkdir -p "$WORK/x"
tar -xjf "$WORK/$NAME" -C "$WORK/x"
SRC="$WORK/x/${NAME%.tar.bz2}"

rm -rf "$DEST"
mv "$SRC/Release" "$DEST"
for item in CMakeLists.txt cmake include libcef_dll CREDITS.html; do mv "$SRC/$item" "$DEST/"; done
printf '{\n  "type": "minimal",\n  "name": "%s",\n  "sha1": "%s"\n}\n' "$NAME" "$SHA1" > "$DEST/archive.json"
echo "CEF installed in $DEST"
