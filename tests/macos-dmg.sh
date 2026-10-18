#!/bin/sh
# Build a compressed disk image, then prove it verifies and mounts.
# The app is ad-hoc signed first. A linker signature without sealed resources
# is what Gatekeeper reports as a damaged download.
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: macos-dmg.sh APP.app OUTPUT.dmg" >&2
  exit 1
fi

app="$1"
output="$2"
test -d "$app"
test -x "$app/Contents/MacOS/token-dock"

work="$(mktemp -d)"
mountpoint=""
cleanup() {
  if [ -n "$mountpoint" ]; then
    /usr/bin/hdiutil detach "$mountpoint" >/dev/null 2>&1 \
      || /usr/bin/hdiutil detach -force "$mountpoint" >/dev/null 2>&1 \
      || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT HUP INT TERM

hdiutil_retry() {
  tries=0
  while [ "$tries" -lt 4 ]; do
    if /usr/bin/hdiutil "$@"; then
      return 0
    fi
    tries=$((tries + 1))
    sleep 2
  done
  return 1
}

detach_mount() {
  point="$1"
  tries=0
  while [ "$tries" -lt 3 ]; do
    if /usr/bin/hdiutil detach "$point"; then
      mountpoint=""
      return 0
    fi
    tries=$((tries + 1))
    sleep 2
  done
  /usr/bin/hdiutil detach -force "$point"
  mountpoint=""
}

stage="$work/stage"
mkdir -p "$stage"
/usr/bin/ditto "$app" "$stage/TokenDock.app"
chmod +x "$stage/TokenDock.app/Contents/MacOS/token-dock"
ln -s /Applications "$stage/Applications"

# Replace the linker ad-hoc signature, which requires a resource seal the
# bundle does not have, with a signature that covers the bundle.
/usr/bin/codesign --force --sign - "$stage/TokenDock.app"
/usr/bin/codesign --verify --deep --strict "$stage/TokenDock.app"
/usr/bin/codesign -dv --verbose=4 "$stage/TokenDock.app" >"$work/signature.txt" 2>&1
cat "$work/signature.txt"
grep -q 'Signature=adhoc' "$work/signature.txt"
grep -q 'Sealed Resources version=' "$work/signature.txt"

# Create a read-write image, detach it, then convert. A one-step UDZO image
# is compressed before that volume is closed.
rw="$work/read-write.dmg"
hdiutil_retry create -volname "TokenDock" -srcfolder "$stage" -ov -fs HFS+ -format UDRW "$rw"
# create can leave the new image attached. Detach that mount before converting.
/usr/bin/hdiutil detach "$rw" >/dev/null 2>&1 || true
mountpoint="$(mktemp -d)"
hdiutil_retry attach "$rw" -nobrowse -readonly -mountpoint "$mountpoint"
test -x "$mountpoint/TokenDock.app/Contents/MacOS/token-dock"
test -L "$mountpoint/Applications"
point="$mountpoint"
detach_mount "$point"
rmdir "$point" 2>/dev/null || true

rm -f "$output"
hdiutil_retry convert "$rw" -format UDZO -imagekey zlib-level=9 -o "$output"
if [ ! -f "$output" ] && [ -f "${output}.dmg" ]; then
  mv "${output}.dmg" "$output"
fi
test -f "$output"
rm -f "$rw"

hdiutil_retry verify "$output"
mountpoint="$(mktemp -d)"
hdiutil_retry attach "$output" -nobrowse -readonly -mountpoint "$mountpoint"
test -d "$mountpoint/TokenDock.app"
test -x "$mountpoint/TokenDock.app/Contents/MacOS/token-dock"
test -L "$mountpoint/Applications"
/usr/bin/codesign --verify --deep --strict "$mountpoint/TokenDock.app"
point="$mountpoint"
detach_mount "$point"
rmdir "$point" 2>/dev/null || true

# Leave the container unsigned. Apple specifies a Developer ID Application
# identity for disk-image signing; an ad-hoc image signature is not trusted.
# The app inside retains its sealed ad-hoc signature and still requires
# first-launch approval. Integrity and mounting were verified above.

echo "Verified and mounted $output"
