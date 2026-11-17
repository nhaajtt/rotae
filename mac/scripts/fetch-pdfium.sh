#!/bin/sh
# Downloads the arm64 PDFium build; the archive must match the pinned hash.
set -eu
TAG="chromium/8138"
SHA256="${PDFIUM_SHA256:-}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/engine/resources/bin"
mkdir -p "$OUT"
[ -f "$OUT/libpdfium.dylib" ] && exit 0
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
curl --retry 5 --retry-all-errors --proto '=https' --proto-redir '=https' -fsSL -o "$T/p.tgz" "https://github.com/bblanchon/pdfium-binaries/releases/download/$TAG/pdfium-mac-arm64.tgz"
GOT="$(shasum -a 256 "$T/p.tgz" | cut -d' ' -f1)"
PIN="$(cat "$HERE/pdfium.sha256" 2>/dev/null || true)"
if [ -n "$SHA256" ]; then PIN="$SHA256"; fi
if [ -z "$PIN" ]; then echo "$GOT" > "$HERE/pdfium.sha256"; echo "pinned $GOT"; elif [ "$GOT" != "$PIN" ]; then echo "hash mismatch: $GOT" >&2; exit 1; fi
tar -xzf "$T/p.tgz" -C "$T"
cp "$T/lib/libpdfium.dylib" "$OUT/"
