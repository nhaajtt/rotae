#!/bin/sh
# Builds build/Rotae.app. Xcode 26 or later gives real Liquid Glass, the layered icon and
# Shortcuts actions; with only the Command Line Tools it still builds, without those.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
scripts/fetch-pdfium.sh
(cd engine && cargo build --release --locked)
swift build -c release --product Rotae
swift build -c release --product sd-native
swift build -c release --product rotae-cli
APP=build/Rotae.app
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources/vi.lproj" "$APP/Contents/Resources/en.lproj"
cp .build/release/Rotae .build/release/sd-native .build/release/rotae-cli engine/target/release/sd-engine "$APP/Contents/MacOS/"
# SwiftPM records the deployment target as the SDK too, and macOS then treats the app as built
# for an old system and withholds the current design (Liquid Glass). Record the SDK really used.
SDKV="$(xcrun --sdk macosx --show-sdk-version)"
if xcrun --find vtool >/dev/null 2>&1; then
  xcrun vtool -set-build-version macos 14.0 "$SDKV" -replace -output "$APP/Contents/MacOS/Rotae" "$APP/Contents/MacOS/Rotae" 2>/dev/null
fi
cp engine/resources/bin/libpdfium.dylib "$APP/Contents/Frameworks/"
cp Resources/Info.plist "$APP/Contents/"
python3 scripts/gen-strings.py ../src/i18n.js Resources/vi-extra.txt "$APP/Contents/Resources/vi.lproj/Localizable.strings"
# The icon: layered Liquid Glass where Xcode can compile it (it also writes the .icns older
# systems use); the flat picture otherwise.
if xcrun --find actool >/dev/null 2>&1 && xcrun actool Resources/AppIcon.icon --compile "$APP/Contents/Resources" --app-icon AppIcon --platform macosx --minimum-deployment-target 14.0 --output-partial-info-plist "$(mktemp -d)/icon.plist" >/dev/null 2>&1 && [ -f "$APP/Contents/Resources/AppIcon.icns" ]; then
  :
else
  rm -f "$APP/Contents/Resources/Assets.car"
  SET="$(mktemp -d)/AppIcon.iconset"; mkdir "$SET"
  for s in 16 32 128 256 512; do
    sips -z $s $s ../brand/icon.png --out "$SET/icon_${s}x${s}.png" >/dev/null
    d=$((s*2)); sips -z $d $d ../brand/icon.png --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$SET" -o "$APP/Contents/Resources/AppIcon.icns"
fi
# Shortcuts and Spotlight find the app's actions through this metadata; Xcode writes it for an
# Xcode project, here it is made from what the compiler already recorded.
CONST="$(find .build -ipath '*release*' -name '*.swiftconstvalues' -path '*Rotae*' 2>/dev/null | grep -v sd- || true)"
if [ -n "$CONST" ] && xcrun --find appintentsmetadataprocessor >/dev/null 2>&1; then
  W="$(mktemp -d)"; ls "$ROOT"/Sources/Rotae/*.swift > "$W/src"; for f in $CONST; do echo "$ROOT/$f"; done > "$W/const"
  xcrun appintentsmetadataprocessor --output "$APP/Contents/Resources" --toolchain-dir "$(dirname "$(dirname "$(dirname "$(xcrun --find swiftc)")")")" \
    --module-name Rotae --sdk-root "$(xcrun --sdk macosx --show-sdk-path)" --xcode-version "$(xcodebuild -version | awk '/Build version/{print $3}')" \
    --platform-family macOS --deployment-target 14.0 --target-triple "$(uname -m)-apple-macos14.0" --source-file-list "$W/src" --swift-const-vals-list "$W/const" \
    --force --deployment-aware-processing --no-app-shortcuts-localization >/dev/null 2>&1 || echo "note: Shortcuts metadata was not generated" >&2
fi
scripts/sign.sh "$APP"
echo "$APP"
