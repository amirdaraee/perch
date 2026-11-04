#!/usr/bin/env bash
# Check a release download the way a user will get it: unzipped or mounted, not
# the bundle in build/ that it was made from.
#
#   scripts/check-release.sh <Perch-x.y.z-macos.zip|.dmg> <adhoc|notarized>
#
# Both binaries must be universal. GitHub's macOS runners are Apple Silicon, so
# an arm64-only slip would produce an app that simply fails to launch on every
# Intel Mac, and nobody would notice until a user reported it. In notarized
# mode it must also carry the Developer ID, a stapled ticket, and pass the same
# Gatekeeper check a user's Mac makes.
set -uo pipefail

FILE=${1:?usage: check-release.sh <zip|dmg> <adhoc|notarized>}
MODE=${2:?usage: check-release.sh <zip|dmg> <adhoc|notarized>}

fail() { echo "::error::$*"; exit 1; }

CHECK=$(mktemp -d)
MOUNTED=
cleanup() {
  # Detached first: rm -rf must never walk into a mounted image.
  if [ -n "$MOUNTED" ]; then
    hdiutil detach -quiet "$CHECK" || hdiutil detach -quiet -force "$CHECK" || return
  fi
  rm -rf "$CHECK"
}
trap cleanup EXIT

case "$FILE" in
  *.dmg)
    hdiutil attach -quiet -readonly -nobrowse -mountpoint "$CHECK" "$FILE" \
      || fail "Could not mount $FILE."
    MOUNTED=1
    [ "$(readlink "$CHECK/Applications")" = /Applications ] \
      || fail "The disk image has no link to /Applications to drag Perch onto."
    ;;
  *.zip)
    ditto -x -k "$FILE" "$CHECK" || fail "Could not unzip $FILE."
    ;;
  *) fail "$FILE is neither a .zip nor a .dmg." ;;
esac
APP="$CHECK/Perch.app"

for bin in Perch perch-mcp; do
  info=$(lipo -info "$APP/Contents/MacOS/$bin") || fail "$bin is missing from the bundle."
  echo "$info"
  for arch in arm64 x86_64; do
    printf '%s' "$info" | grep -qw "$arch" || fail "$bin is missing the $arch slice."
  done
  # Captured, not piped into `grep -q`: under pipefail, grep exiting at the
  # first match makes codesign die of SIGPIPE and the check fail on success.
  sig=$(codesign -d --verbose=2 "$APP/Contents/MacOS/$bin" 2>&1)
  grep -q 'flags=.*runtime' <<<"$sig" || fail "$bin is not signed with the hardened runtime."
done

codesign --verify --deep --strict "$APP" || fail "The signature does not verify."

case "$MODE" in
  adhoc) ;;
  notarized)
    sig=$(codesign -dv --verbose=2 "$APP" 2>&1)
    grep -q '^Authority=Developer ID Application' <<<"$sig" \
      || fail "The app is not signed with the Developer ID."
    xcrun stapler validate "$APP" || fail "The app has no stapled notarization ticket."
    spctl --assess --type execute --verbose=2 "$APP" || fail "Gatekeeper rejects the app."
    case "$FILE" in
      *.dmg)
        sig=$(codesign -dv --verbose=2 "$FILE" 2>&1)
        grep -q '^Authority=Developer ID Application' <<<"$sig" \
          || fail "The disk image is not signed with the Developer ID."
        xcrun stapler validate "$FILE" || fail "The disk image has no stapled notarization ticket."
        spctl --assess --type open --context context:primary-signature --verbose=2 "$FILE" \
          || fail "Gatekeeper rejects the disk image."
        ;;
    esac
    ;;
  *) fail "Unknown mode '$MODE'." ;;
esac

echo "$FILE looks right for this release ($MODE)."
