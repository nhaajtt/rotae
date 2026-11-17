#!/bin/sh
# build/Rotae.dmg with an Applications shortcut.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
S="$(mktemp -d)"; cp -R build/Rotae.app "$S/"; ln -s /Applications "$S/Applications"
rm -f build/Rotae.dmg
hdiutil create -quiet -volname Rotae -srcfolder "$S" -format UDZO build/Rotae.dmg
rm -rf "$S"
if [ -n "${NOTARY_PROFILE:-}" ]; then
  xcrun notarytool submit build/Rotae.dmg --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple build/Rotae.dmg
fi
