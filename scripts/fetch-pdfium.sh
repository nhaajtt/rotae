#!/usr/bin/env sh
# Downloads libpdfium.so (Linux x64) from bblanchon/pdfium-binaries into src-tauri/resources/bin.
set -eu
dest="$(dirname "$0")/../src-tauri/resources/bin"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
url="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz"
echo "Downloading $url"
curl -fsSL --proto '=https' "$url" -o "$tmp/pdfium.tgz"
tar -xzf "$tmp/pdfium.tgz" -C "$tmp"
mkdir -p "$dest"
cp "$tmp/lib/libpdfium.so" "$dest/"
echo "libpdfium.so -> $dest"
