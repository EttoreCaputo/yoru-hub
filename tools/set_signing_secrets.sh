#!/usr/bin/env bash
# Give the release workflow what it needs to sign the app with the Developer
# ID certificate of team 79AC9KV62T and to have Apple notarize it. Run it
# yourself, in a terminal: it asks for the passwords and hands them straight
# to `gh secret set`, so they never land in a file or in the shell history.
#
#   tools/set_signing_secrets.sh path/to/DeveloperID.p12
#
# Before it:
# 1. Xcode → Settings → Accounts → the team 79AC9KV62T → Manage Certificates
#    → + → Developer ID Application. The certificate and its key land in the
#    login keychain.
# 2. Keychain Access → My Certificates → "Developer ID Application: …" →
#    Export (.p12) with a password. Only that one: the workflow must not get
#    the other certificates.
# 3. An app-specific password for notarization, made at account.apple.com
#    → Sign-In and Security → App-Specific Passwords.

set -euo pipefail

team=79AC9KV62T
p12="${1:-}"
if [[ -z "$p12" || ! -f "$p12" ]]; then
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
fi
gh auth status > /dev/null

read -r -s -p "Password of $(basename "$p12"): " p12_password; echo
# The certificate's name, e.g. "Developer ID Application: Ettore Caputo (79AC9KV62T)".
certificate() {
    # An older Keychain Access exports with RC2/3DES, which OpenSSL 3 reads only with -legacy.
    P="$p12_password" openssl pkcs12 -in "$p12" -nokeys -passin env:P 2> /dev/null ||
        P="$p12_password" openssl pkcs12 -legacy -in "$p12" -nokeys -passin env:P 2> /dev/null
}
identity="$(certificate |
    sed -n 's/^subject=.*CN *= *\(Developer ID Application:[^,/]*\).*/\1/p' | head -1)"
if [[ -z "$identity" ]]; then
    echo "yoru: no Developer ID Application certificate in $p12, or a wrong password" >&2
    exit 1
fi
if [[ "$identity" != *"($team)"* ]]; then
    echo "yoru: $identity is not of team $team" >&2
    exit 1
fi
echo "yoru: $identity"

read -r -p "Apple ID (email) for notarization: " apple_id
read -r -s -p "App-specific password: " apple_password; echo

base64 -i "$p12" | tr -d '\n' | gh secret set APPLE_CERTIFICATE
printf '%s' "$p12_password" | gh secret set APPLE_CERTIFICATE_PASSWORD
printf '%s' "$identity" | gh secret set APPLE_SIGNING_IDENTITY
printf '%s' "$apple_id" | gh secret set APPLE_ID
printf '%s' "$apple_password" | gh secret set APPLE_PASSWORD
printf '%s' "$team" | gh secret set APPLE_TEAM_ID
echo "yoru: secrets set; the next push to main or develop builds a signed, notarized app"
