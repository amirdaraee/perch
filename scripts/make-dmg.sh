#!/usr/bin/env bash
# Wrap a signed Perch.app in a disk image with the usual drag-to-Applications
# link, and sign the image too when given a real identity.
#
#   scripts/make-dmg.sh <path/to/Perch.app> <path/to/output.dmg> [identity]
#
# Built with hdiutil alone. Laying out icons over a background picture means
# scripting Finder, which needs a logged-in GUI session and is unreliable on CI.
set -euo pipefail

APP=${1:?usage: make-dmg.sh <Perch.app> <output.dmg> [identity]}
DMG=${2:?usage: make-dmg.sh <Perch.app> <output.dmg> [identity]}
IDENTITY=${3:--}

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
# The image's root takes this folder's mode, and mktemp's is private.
chmod 755 "$STAGE"

ditto "$APP" "$STAGE/Perch.app"
ln -s /Applications "$STAGE/Applications"

# hdiutil fails now and then with "Resource busy" on GitHub's runners, when
# something is still scanning the files it was just handed.
rm -f "$DMG"
for attempt in 1 2 3; do
  hdiutil create -quiet -volname Perch -srcfolder "$STAGE" -format UDZO -ov "$DMG" && break
  [ "$attempt" -lt 3 ] || { echo "hdiutil failed three times." >&2; exit 1; }
  sleep 10
done

if [ "$IDENTITY" != "-" ]; then
  codesign --force --timestamp --sign "$IDENTITY" "$DMG"
  codesign --verify --strict "$DMG"
fi

shasum -a 256 "$DMG"
