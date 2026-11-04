#!/usr/bin/env bash
# Notarize and staple a Developer ID-signed Perch.app or disk image.
#
#   scripts/notarize.sh <path/to/Perch.app> <path/to/output.zip>
#   scripts/notarize.sh <path/to/Perch.dmg>
#
# The app comes first, so the copy inside the disk image carries its own ticket
# and still opens on a Mac that is offline. Build the image from the stapled
# app, then notarize the image as well.
#
# Authenticates with an App Store Connect API key, read from the environment:
#   NOTARY_KEY_PATH   path to the AuthKey_XXXXXXXXXX.p8 file
#   NOTARY_KEY_ID     the key's ID (10 characters)
#   NOTARY_ISSUER_ID  the issuer UUID shown above the keys list
#
# An API key rather than an Apple ID and app-specific password: it can be
# revoked on its own, it isn't tied to a person's login, and it doesn't trip
# two-factor prompts in CI.
set -euo pipefail

USAGE="usage: notarize.sh <Perch.app> <output.zip> | notarize.sh <Perch.dmg>"
TARGET=${1:?$USAGE}
: "${NOTARY_KEY_PATH:?NOTARY_KEY_PATH is not set}"
: "${NOTARY_KEY_ID:?NOTARY_KEY_ID is not set}"
: "${NOTARY_ISSUER_ID:?NOTARY_ISSUER_ID is not set}"

auth=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")

submit() {
  echo "Submitting $1 to Apple. This usually takes a few minutes."
  local result id status
  result=$(xcrun notarytool submit "$1" "${auth[@]}" \
             --wait --timeout 45m --output-format json)
  echo "$result"

  id=$(plutil -extract id raw - <<<"$result" 2>/dev/null || true)
  status=$(plutil -extract status raw - <<<"$result" 2>/dev/null || true)

  if [ "$status" != "Accepted" ]; then
    # The log is where Apple says *which* binary failed and why. Without it a
    # rejection is just the word "Invalid".
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" "${auth[@]}" || true
    fi
    echo "Notarization was not accepted (status: ${status:-unknown})." >&2
    exit 1
  fi
}

case "$TARGET" in
  *.dmg)
    submit "$TARGET"
    xcrun stapler staple "$TARGET"
    xcrun stapler validate "$TARGET"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$TARGET"
    echo "Notarized and stapled: $TARGET"
    ;;
  *.app)
    ZIP=${2:?$USAGE}

    # Apple is sent a zip of the app; the ticket it issues is then stapled to
    # the app itself, so the zip handed to users has to be made again afterwards.
    SUBMISSION="$(dirname "$ZIP")/notarize-submission.zip"
    rm -f "$SUBMISSION"
    ditto -c -k --keepParent "$TARGET" "$SUBMISSION"
    submit "$SUBMISSION"
    rm -f "$SUBMISSION"

    # Stapling puts the ticket inside the app, so Gatekeeper can check it on a
    # Mac that is offline at first launch.
    xcrun stapler staple "$TARGET"
    xcrun stapler validate "$TARGET"

    # The check a user's Mac will make. If this passes, there is no "Open Anyway".
    spctl --assess --type execute --verbose=2 "$TARGET"

    rm -f "$ZIP"
    ditto -c -k --keepParent "$TARGET" "$ZIP"
    echo "Notarized and stapled: $ZIP"
    ;;
  *)
    echo "$USAGE" >&2
    exit 1
    ;;
esac
