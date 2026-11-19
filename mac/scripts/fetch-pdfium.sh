#!/bin/sh
# Downloads the arm64 PDFium build; the archive must match the pinned hash, and a copy left from
# an earlier run is only reused when it still matches the dylib that came out of that archive.
set -eu
TAG="chromium/8138"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/engine/resources/bin"
PIN="${PDFIUM_SHA256:-$(cat "$HERE/pdfium.sha256" 2>/dev/null || true)}"
[ -n "$PIN" ] || { echo "no pinned PDFium hash (mac/pdfium.sha256 or PDFIUM_SHA256)" >&2; exit 1; }
mkdir -p "$OUT"
LIB="$OUT/libpdfium.dylib"
STAMP="$OUT/libpdfium.dylib.sha256"
if [ -f "$LIB" ] && [ -f "$STAMP" ] && [ "$(sed -n 1p "$STAMP")" = "$PIN" ] \
  && [ "$(shasum -a 256 "$LIB" | cut -d' ' -f1)" = "$(sed -n 2p "$STAMP")" ]; then
  exit 0
fi
rm -f "$LIB" "$STAMP"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
curl --retry 5 --retry-all-errors --proto '=https' --proto-redir '=https' -fsSL -o "$T/p.tgz" "https://github.com/bblanchon/pdfium-binaries/releases/download/$TAG/pdfium-mac-arm64.tgz"
GOT="$(shasum -a 256 "$T/p.tgz" | cut -d' ' -f1)"
[ "$GOT" = "$PIN" ] || { echo "hash mismatch: $GOT" >&2; exit 1; }
tar -xzf "$T/p.tgz" -C "$T"
cp "$T/lib/libpdfium.dylib" "$LIB"
printf '%s\n%s\n' "$PIN" "$(shasum -a 256 "$LIB" | cut -d' ' -f1)" > "$STAMP"
