#!/bin/sh
# Rotae installer for Linux (x86-64) and macOS (Apple silicon).
#
#   curl -fsSL https://rotae.nhaajt.com/install.sh | sh
#
# macOS: puts Rotae.app in /Applications (or ~/Applications when that isn't writable) and the
# `rotae` command in ~/.local/bin. No "unidentified developer" prompt: macOS only checks apps that
# carry the "downloaded from the internet" mark, which browsers add and curl doesn't. The SHA-256
# and code signature checks below stand in for that check.
#   sh install.sh --uninstall   removes the Mac app and the command (settings stay)
#
# What it does: asks GitHub for the newest Rotae release and picks the package that fits this
# system: .deb on Debian, Ubuntu and Kali, .rpm on Fedora and other dnf/zypper systems, and the
# AppImage anywhere else (into ~/.local/bin, no root). It checks the download's SHA-256 against
# the one GitHub publishes before installing. Run it again to update.
#
#   sh install.sh --appimage   use the AppImage even where a package would fit
#   sh install.sh --pentest    also install the common tools for Pentest user mode (apt only)

set -eu

REPO="nhaajtt/rotae"
# Apple Team ID of the Developer ID certificate that signs the Mac app. Once it is set, only an app
# signed by that team and accepted by Gatekeeper gets installed; while empty (ad-hoc builds) the
# app is installed with Gatekeeper's quarantine left in place, so macOS still asks before it runs.
MAC_TEAM_ID=""

# --- macOS ---------------------------------------------------------------------------------------
mac_say() { printf 'Rotae: %s\n' "$*"; }
mac_fail() { printf 'Rotae was not installed: %s\n' "$*" >&2; exit 1; }

mac_link_cli() {
  bin="$HOME/.local/bin"; link="$bin/rotae"
  mkdir -p "$bin"
  # only ever replace a link, never a real file someone put there
  if [ -L "$link" ] || [ ! -e "$link" ]; then
    ln -sfn "$1/Rotae.app/Contents/MacOS/rotae-cli" "$link"
    case ":$PATH:" in
      *":$bin:"*) mac_say "type rotae in a terminal to use it from the command line." ;;
      *) mac_say "for the rotae command, add ~/.local/bin to your PATH:"
         mac_say "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc" ;;
    esac
  fi
}

mac_uninstall() {
  pkill -x Rotae 2>/dev/null || true
  found=""
  for d in /Applications "$HOME/Applications"; do
    if [ -d "$d/Rotae.app" ]; then rm -rf "$d/Rotae.app" && found=1 && mac_say "removed $d/Rotae.app"; fi
  done
  link="$HOME/.local/bin/rotae"
  if [ -L "$link" ] && readlink "$link" | grep -q 'Rotae\.app'; then rm -f "$link"; mac_say "removed $link"; fi
  [ -n "$found" ] || mac_say "Rotae wasn't installed."
  mac_say "your settings stay in ~/Library/Application Support/Rotae; delete that folder to remove them too."
}

install_mac() {
  case "${1:-}" in
    --uninstall) mac_uninstall; return ;;
    "") ;;
    *) mac_fail "unknown option $1 (on a Mac the only one is --uninstall)" ;;
  esac
  [ "$(uname -m)" = arm64 ] || mac_fail "Rotae for Mac needs Apple silicon (M1 or later)."
  major="$(sw_vers -productVersion | cut -d. -f1)"
  [ "$major" -ge 14 ] || mac_fail "Rotae needs macOS 14 or later (this Mac has $(sw_vers -productVersion))."
  if [ -d /Applications ] && [ -w /Applications ]; then dest=/Applications; else dest="$HOME/Applications"; fi

  # releases/latest/download/ is a plain redirect, so no API call (and no API rate limit)
  url="https://github.com/$REPO/releases/latest/download/Rotae-mac-arm64.zip"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT INT TERM

  mac_say "downloading the newest version..."
  curl -fL --proto '=https' --proto-redir '=https' --retry 2 --progress-bar -o "$tmp/Rotae.zip" "$url" \
    || mac_fail "the newest release has no Mac build yet, or GitHub didn't answer. Try again in a few minutes."
  curl -fsSL --proto '=https' --proto-redir '=https' -o "$tmp/Rotae.zip.sha256" "$url.sha256" || mac_fail "couldn't get the checksum."
  expected="$(awk '{print $1; exit}' "$tmp/Rotae.zip.sha256" | tr 'A-F' 'a-f')"
  actual="$(shasum -a 256 "$tmp/Rotae.zip" | awk '{print $1}')"
  [ ${#expected} -eq 64 ] && [ "$expected" = "$actual" ] || mac_fail "SHA-256 does not match. Nothing was installed."
  mac_say "SHA-256 matches."

  ditto -x -k "$tmp/Rotae.zip" "$tmp/out"
  [ -d "$tmp/out/Rotae.app" ] || mac_fail "the download doesn't contain Rotae.app."
  codesign --verify --deep --strict "$tmp/out/Rotae.app" 2>/dev/null || mac_fail "the app's signature doesn't verify. Nothing was installed."
  team="$(codesign -dv "$tmp/out/Rotae.app" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
  trusted=""
  if [ -n "$MAC_TEAM_ID" ]; then
    [ "$team" = "$MAC_TEAM_ID" ] || mac_fail "the app isn't signed by the Rotae developer (team ${team:-none}). Nothing was installed."
    spctl -a -t exec "$tmp/out/Rotae.app" 2>/dev/null || mac_fail "Gatekeeper doesn't accept this app. Nothing was installed."
    trusted=1
  fi
  version="$(plutil -extract CFBundleShortVersionString raw "$tmp/out/Rotae.app/Contents/Info.plist" 2>/dev/null || true)"

  # quit a running copy so it can be replaced
  if pgrep -xq Rotae; then
    mac_say "closing Rotae..."
    pkill -x Rotae || true
    i=0; while pgrep -xq Rotae && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
  fi

  mkdir -p "$dest"
  # the old copy only goes once the new one is fully in place
  rm -rf "$dest/.Rotae.app.new"
  ditto "$tmp/out/Rotae.app" "$dest/.Rotae.app.new"
  if [ -n "$trusted" ]; then
    xattr -dr com.apple.quarantine "$dest/.Rotae.app.new" 2>/dev/null || true
  else
    # not signed by a known developer: let Gatekeeper ask the user on first open
    xattr -w com.apple.quarantine "0081;$(printf '%x' "$(date +%s)");install.sh;" "$dest/.Rotae.app.new" 2>/dev/null || true
  fi
  rm -rf "$dest/Rotae.app"
  mv "$dest/.Rotae.app.new" "$dest/Rotae.app"

  mac_link_cli "$dest"
  mac_say "${version:+$version }is installed in $dest. It lives in the menu bar."
  mac_say "To use it: in Finder, press on a file, hold Shift and drag onto the wheel that opens."
  [ -n "$trusted" ] || mac_say "macOS will ask before the first start: open System Settings > Privacy & Security and choose Open Anyway."
  open "$dest/Rotae.app"
}

if [ "$(uname -s)" = "Darwin" ]; then
  install_mac "$@"
  exit 0
fi
API="https://api.github.com/repos/$REPO/releases/latest"
PREFIX="https://github.com/$REPO/releases/download/"
WANT_APPIMAGE=0
PENTEST=0
for a in "$@"; do
  case "$a" in
    --appimage) WANT_APPIMAGE=1 ;;
    --pentest) PENTEST=1 ;;
    *) echo "Rotae: unknown option $a" >&2; exit 2 ;;
  esac
done

say() { printf 'Rotae: %s\n' "$*"; }
fail() { printf 'Rotae was not installed: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

[ "$(uname -s)" = "Linux" ] || fail "this installer is for Linux (Windows: see rotae.nhaajt.com)."
case "$(uname -m)" in x86_64 | amd64) ;; *) fail "Rotae for Linux is built for x86-64 only, this is $(uname -m)." ;; esac
have curl || fail "curl is needed (sudo apt install curl)."
have sha256sum || fail "sha256sum is needed (coreutils)."

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  if have sudo; then SUDO="sudo"; fi
fi

kind="appimage"
if [ "$WANT_APPIMAGE" -eq 0 ]; then
  if have apt-get && have dpkg; then kind="deb"
  elif have dnf || have zypper; then kind="rpm"
  fi
fi
if [ "$kind" != "appimage" ] && [ -z "$SUDO" ] && [ "$(id -u)" -ne 0 ]; then
  say "no sudo here, using the AppImage (installs for you only)."
  kind="appimage"
fi
case "$kind" in
  deb) pattern='_amd64\.deb$' ;;
  rpm) pattern='\.x86_64\.rpm$' ;;
  *) pattern='_amd64\.AppImage$' ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "looking up the newest release..."
curl -fsSL --proto '=https' -H 'User-Agent: rotae-install' "$API" -o "$tmp/release.json" || fail "GitHub didn't answer. Check the internet connection."

# name, address and SHA-256 of the asset whose name matches; python3 when present, else plain text tools
pick() {
  if have python3; then
    python3 - "$tmp/release.json" "$pattern" <<'PY'
import json, re, sys
r = json.load(open(sys.argv[1]))
for a in r.get("assets", []):
    if re.search(sys.argv[2], a.get("name", "")):
        print(r.get("tag_name", ""))
        print(a["name"]); print(a["browser_download_url"]); print(a.get("digest") or "")
        break
PY
  else
    tr ',' '\n' < "$tmp/release.json" | grep -E '"(tag_name|name|browser_download_url|digest)"' | sed -E 's/^[^:]*: *"?([^"]*)"?.*$/\1/' |
      awk -v p="$pattern" 'NR==1{tag=$0;next} {buf[++n]=$0} END{for(i=1;i<=n;i++) if(buf[i] ~ p){print tag; print buf[i]; for(j=i+1;j<=i+3&&j<=n;j++){ if(buf[j] ~ /^https:/) url=buf[j]; if(buf[j] ~ /^sha256:/) d=buf[j] } print url; print d; exit}}'
  fi
}
info="$(pick)" || true
tag="$(printf '%s\n' "$info" | sed -n 1p)"
name="$(printf '%s\n' "$info" | sed -n 2p)"
url="$(printf '%s\n' "$info" | sed -n 3p)"
digest="$(printf '%s\n' "$info" | sed -n 4p)"
[ -n "$name" ] && [ -n "$url" ] || fail "the newest release has no Linux $kind package yet."
case "$url" in "$PREFIX"*) ;; *) fail "unexpected download address: $url" ;; esac
case "$name" in */* | *..*) fail "unexpected file name: $name" ;; esac
expected="$(printf '%s' "$digest" | sed -n 's/^sha256:\([0-9a-fA-F]\{64\}\)$/\1/p' | tr 'A-F' 'a-f')"
[ -n "$expected" ] || fail "the release doesn't publish a SHA-256, so the download can't be checked."

say "downloading $tag ($name)..."
curl -fL --proto '=https' --proto-redir '=https' --retry 2 -o "$tmp/$name" "$url" || fail "the download didn't complete."
actual="$(sha256sum "$tmp/$name" | cut -d' ' -f1)"
[ "$actual" = "$expected" ] || fail "SHA-256 does not match (expected $expected, got $actual). Nothing was installed."
say "SHA-256 matches."

case "$kind" in
  deb)
    say "installing with apt (it may ask for your password)..."
    # apt pulls in what Rotae uses: FFmpeg, Tesseract, bubblewrap, fonts...
    $SUDO apt-get install -y "$tmp/$name" || fail "apt couldn't install the package."
    ;;
  rpm)
    say "installing the package (it may ask for your password)..."
    if have dnf; then $SUDO dnf install -y "$tmp/$name" || fail "dnf couldn't install the package."
    else $SUDO zypper --non-interactive install --allow-unsigned-rpm "$tmp/$name" || fail "zypper couldn't install the package."
    fi
    ;;
  appimage)
    bin="$HOME/.local/bin"; apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    mkdir -p "$bin" "$apps"
    install -m 0755 "$tmp/$name" "$bin/Rotae.AppImage"
    cat > "$apps/rotae.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Rotae
Comment=Drag a file to convert, compress, check and share it
Exec="$bin/Rotae.AppImage"
Icon=rotae
Terminal=false
Categories=Utility;
EOF
    say "installed to $bin/Rotae.AppImage"
    ;;
esac

if [ "$PENTEST" -eq 1 ]; then
  if have apt-get && [ -n "$SUDO$( [ "$(id -u)" -eq 0 ] && echo root)" ]; then
    say "installing the tools Pentest user mode uses (whatever this distribution has)..."
    for p in bubblewrap libimage-exiftool-perl binwalk foremost ssdeep radare2 binutils tshark hcxtools hashid john \
      steghide pngcheck pdfid pdf-parser apktool jadx clamav nmap subfinder dnsx httpx-toolkit nuclei sslscan; do
      $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y "$p" >/dev/null 2>&1 && printf '  + %s\n' "$p" || printf '  - %s (not in this distribution)\n' "$p"
    done
  else
    say "--pentest needs apt (Kali, Debian, Ubuntu); install the tools from Settings > Pentest user mode instead."
  fi
fi

say "$tag is installed. Open it from your apps menu; it lives in the system tray."
say "To use it: drag a file to the glowing strip at the right edge of the screen (on X11 you can also hold Shift while dragging)."
