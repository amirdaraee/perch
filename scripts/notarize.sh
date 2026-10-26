#!/usr/bin/env bash
# Notarize, staple and re-zip a Developer ID-signed Perch.app.
#
#   scripts/notarize.sh <path/to/Perch.app> <path/to/output.zip>
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

APP=${1:?usage: notarize.sh <Perch.app> <output.zip>}
ZIP=${2:?usage: notarize.sh <Perch.app> <output.zip>}
: "${NOTARY_KEY_PATH:?NOTARY_KEY_PATH is not set}"
: "${NOTARY_KEY_ID:?NOTARY_KEY_ID is not set}"
: "${NOTARY_ISSUER_ID:?NOTARY_ISSUER_ID is not set}"

auth=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")

# Apple is sent a zip of the app; the ticket it issues is then stapled to the
# app itself, so the zip handed to users has to be made again afterwards.
SUBMISSION="$(dirname "$ZIP")/notarize-submission.zip"
rm -f "$SUBMISSION"
ditto -c -k --keepParent "$APP" "$SUBMISSION"

echo "Submitting to Apple. This usually takes a few minutes."
RESULT=$(xcrun notarytool submit "$SUBMISSION" "${auth[@]}" \
           --wait --timeout 45m --output-format json)
rm -f "$SUBMISSION"
echo "$RESULT"

ID=$(plutil -extract id raw - <<<"$RESULT" 2>/dev/null || true)
STATUS=$(plutil -extract status raw - <<<"$RESULT" 2>/dev/null || true)

if [ "$STATUS" != "Accepted" ]; then
  # The log is where Apple says *which* binary failed and why. Without it a
  # rejection is just the word "Invalid".
  if [ -n "$ID" ]; then
    xcrun notarytool log "$ID" "${auth[@]}" || true
  fi
  echo "Notarization was not accepted (status: ${STATUS:-unknown})." >&2
  exit 1
fi

# Stapling puts the ticket inside the app, so Gatekeeper can check it on a Mac
# that is offline at first launch.
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"

# The check a user's Mac will make. If this passes, there is no "Open Anyway".
spctl --assess --type execute --verbose=2 "$APP"

rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "Notarized and stapled: $ZIP"
