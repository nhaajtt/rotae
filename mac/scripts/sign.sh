#!/bin/sh
# Inside-out signing. SIGN_IDENTITY: "-" (ad-hoc, default) or a Developer ID certificate name.
# With Developer ID, set NOTARY_PROFILE (from `notarytool store-credentials`) to notarize.
set -eu
APP="$1"; ID="${SIGN_IDENTITY:--}"
OPTS="--force --sign"
EXTRA=""
[ "$ID" != "-" ] && EXTRA="--options runtime --timestamp"
codesign $OPTS "$ID" $EXTRA "$APP/Contents/Frameworks/libpdfium.dylib"
codesign $OPTS "$ID" $EXTRA "$APP/Contents/MacOS/sd-native"
codesign $OPTS "$ID" $EXTRA "$APP/Contents/MacOS/sd-engine"
codesign $OPTS "$ID" $EXTRA "$APP/Contents/MacOS/rotae-cli"
codesign $OPTS "$ID" $EXTRA "$APP"
codesign --verify --strict --verbose=1 "$APP"
