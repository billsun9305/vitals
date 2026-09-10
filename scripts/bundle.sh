#!/usr/bin/env bash
# Pack target/release/vitals into dist/Vitals.app and sign it.
#
# SIGN_IDENTITY selects the signature. Unset or "-" is ad-hoc: what a local
# build and a fork without secrets get, and what `make install-app`
# installs. Any other value is a codesign identity such as
# "Developer ID Application: Name (TEAMID)"; the bundle is then signed
# with the hardened runtime and a trusted timestamp so that
# .github/workflows/release.yml can notarize it. There is no entitlements
# file, because the app needs no entitlement.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app="$root/dist/Vitals.app"
identity="${SIGN_IDENTITY:--}"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/resources/Info.plist" "$app/Contents/Info.plist"
cp "$root/target/release/vitals" "$app/Contents/MacOS/vitals"
if [ -f "$root/resources/AppIcon.icns" ]; then
  cp "$root/resources/AppIcon.icns" "$app/Contents/Resources/"
fi

if [ "$identity" = "-" ]; then
  codesign --force --deep --sign - "$app"
else
  codesign --force --deep --options runtime --timestamp --sign "$identity" "$app"
fi
codesign --verify --verbose "$app"
echo "built $app (signed as: $identity)"
