#!/usr/bin/env bash
# Submit one file (a zip of the app, or the DMG) to Apple's notarization
# service and wait for the verdict, then print Apple's own log for the
# submission so a rejection is diagnosable from the CI log alone.
#
#   scripts/notarize.sh <file>
#
# Reads NOTARY_KEY_P8_PATH (the AuthKey .p8 on disk), NOTARY_KEY_ID and
# NOTARY_ISSUER_ID from the environment — .github/workflows/release.yml sets
# them from the repository secrets. Exits 0 only on "Accepted". The wait
# is capped: Apple usually answers within minutes, and a submission that
# is still pending after NOTARY_TIMEOUT (default 45m) fails here with its
# id printed, rather than holding the job for hours.
set -euo pipefail

file="${1:?usage: notarize.sh <file>}"
: "${NOTARY_KEY_P8_PATH:?}" "${NOTARY_KEY_ID:?}" "${NOTARY_ISSUER_ID:?}"
auth=(--key "$NOTARY_KEY_P8_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")

# `submit --wait` exits non-zero on Invalid and on a timeout alike; the JSON
# it prints carries the id and status either way, so capture rather than fail.
out="$(xcrun notarytool submit "$file" "${auth[@]}" --wait --timeout "${NOTARY_TIMEOUT:-45m}" --output-format json 2>&1)" || true
id="$(printf '%s' "$out" | jq -r 'try .id // empty' 2>/dev/null || true)"
status="$(printf '%s' "$out" | jq -r 'try .status // empty' 2>/dev/null || true)"

echo "notarization of $(basename "$file"): ${status:-no verdict} (submission ${id:-unknown})"
if [ -z "$id" ]; then
  echo "$out" >&2
  exit 1
fi
if [ "$status" != "Accepted" ]; then
  echo "--- Apple's log for $id:" >&2
  xcrun notarytool log "$id" "${auth[@]}" >&2 || true
  exit 1
fi
