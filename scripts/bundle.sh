#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app="$root/dist/Vitals.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/resources/Info.plist" "$app/Contents/Info.plist"
cp "$root/target/release/vitals" "$app/Contents/MacOS/vitals"
if [ -f "$root/resources/AppIcon.icns" ]; then
  cp "$root/resources/AppIcon.icns" "$app/Contents/Resources/"
fi

# Ad-hoc signature: sufficient for a local install, and never notarized
# because this binary uses a private API and will never leave this machine.
codesign --force --deep --sign - "$app"
codesign --verify --verbose "$app"
echo "built $app"
