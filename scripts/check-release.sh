#!/usr/bin/env bash
# Check the release zip the way a user will get it: unzipped, not the bundle
# in build/ that it was made from.
#
#   scripts/check-release.sh <Perch-x.y.z-macos.zip> <adhoc|notarized>
#
# Both binaries must be universal. GitHub's macOS runners are Apple Silicon, so
# an arm64-only slip would produce an app that simply fails to launch on every
# Intel Mac, and nobody would notice until a user reported it. In notarized
# mode it must also carry the Developer ID, a stapled ticket, and pass the same
# Gatekeeper check a user's Mac makes.
set -uo pipefail

ZIP=${1:?usage: check-release.sh <zip> <adhoc|notarized>}
MODE=${2:?usage: check-release.sh <zip> <adhoc|notarized>}

fail() { echo "::error::$*"; exit 1; }

CHECK=$(mktemp -d)
trap 'rm -rf "$CHECK"' EXIT
ditto -x -k "$ZIP" "$CHECK" || fail "Could not unzip $ZIP."
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
    ;;
  *) fail "Unknown mode '$MODE'." ;;
esac

echo "$ZIP looks right for this release ($MODE)."
