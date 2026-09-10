#!/usr/bin/env bash
# Set the five repository secrets `.github/workflows/release.yml` signs and
# notarizes with. Run this yourself, in your own terminal: it reads the
# certificate file and the API key from disk and hands them to `gh secret
# set`, and nothing is printed or stored anywhere else.
#
# Before running it you need two things that only the account holder can
# create:
#
#   1. A "Developer ID Application" certificate, exported as a .p12 file.
#      Xcode > Settings > Accounts > (your Apple ID) > Manage Certificates
#      > "+" > Developer ID Application. Then in Keychain Access, under
#      "My Certificates", right-click "Developer ID Application: <name>
#      (<TEAMID>)" > Export, choose the .p12 format, and give it a
#      password. Keychain Access asks for your login password to allow
#      the private key out.
#
#   2. An App Store Connect API key with the "Developer" role.
#      https://appstoreconnect.apple.com > Users and Access > Integrations
#      > App Store Connect API > Team Keys > "+". Note the Key ID and the
#      Issuer ID shown on that page, and download the AuthKey_<KEYID>.p8
#      file (offered exactly once).
set -euo pipefail

repo="billsun9305/vitals"

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1" >&2; exit 1; }; }
need gh
need base64

echo "This sets 5 secrets on github.com/$repo as the active gh account:"
gh auth status 2>&1 | grep -E "Active account|Logged in to" | head -2
echo

# Refuse quietly if the certificate is not there yet, with the fix.
if ! security find-identity -v -p codesigning 2>/dev/null | grep -q "Developer ID Application"; then
  echo "No 'Developer ID Application' certificate is in your keychain yet." >&2
  echo "Create one first (step 1 in the comment at the top of this script), then re-run." >&2
  exit 1
fi

# The repository's git-ignored .secrets/ directory is where the two files
# are kept between releases; each prompt defaults to the one file found
# there, so a repeat run is four Returns and the .p12 password.
secrets_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.secrets"
default_p12="$(ls "$secrets_dir"/*.p12 2>/dev/null | head -1 || true)"
default_p8="$(ls "$secrets_dir"/AuthKey_*.p8 2>/dev/null | head -1 || true)"

read -rp "Path to the exported certificate (.p12) [${default_p12:-none}]: " p12
p12="${p12:-$default_p12}"
[ -f "$p12" ] || { echo "not a file: $p12" >&2; exit 1; }
read -rsp "Password you gave the .p12 when exporting: " p12_password; echo
read -rp "Path to the App Store Connect key (AuthKey_XXXXXXXXXX.p8) [${default_p8:-none}]: " p8
p8="${p8:-$default_p8}"
[ -f "$p8" ] || { echo "not a file: $p8" >&2; exit 1; }
key_id_guess="$(basename "$p8" | sed -nE 's/^AuthKey_([A-Z0-9]+)\.p8$/\1/p')"
read -rp "Key ID [${key_id_guess:-none}]: " key_id
key_id="${key_id:-$key_id_guess}"
[ -n "$key_id" ] || { echo "a Key ID is required" >&2; exit 1; }
read -rp "Issuer ID (the UUID shown above the keys table): " issuer_id
[ -n "$issuer_id" ] || { echo "an Issuer ID is required" >&2; exit 1; }

echo
echo "Setting secrets..."
base64 -i "$p12" | gh secret set MACOS_CERT_P12 --repo "$repo"
printf '%s' "$p12_password" | gh secret set MACOS_CERT_PASSWORD --repo "$repo"
printf '%s' "$key_id" | gh secret set NOTARY_KEY_ID --repo "$repo"
printf '%s' "$issuer_id" | gh secret set NOTARY_ISSUER_ID --repo "$repo"
gh secret set NOTARY_KEY_P8 --repo "$repo" < "$p8"

echo
gh secret list --repo "$repo"
echo
echo "Done. The next tagged release will be signed and notarized."
