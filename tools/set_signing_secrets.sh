#!/usr/bin/env bash
# Give the release workflow what it needs to sign the app with the Developer
# ID certificate of team 79AC9KV62T and to have Apple notarize it. Run it
# yourself, in a terminal: it asks for the passwords and hands them straight
# to `gh secret set`, so they never land in a file or in the shell history.
#
#   tools/set_signing_secrets.sh DeveloperID.key developerID_application.cer
#   tools/set_signing_secrets.sh DeveloperID.p12
#
# Before it, the certificate. Without Keychain Access or Xcode:
# 1. A key and a request:
#      openssl req -new -newkey rsa:2048 -nodes -keyout DeveloperID.key \
#        -out DeveloperID.certSigningRequest -subj "/CN=<your name>/C=IT"
#    Keep the key private and out of the repository.
# 2. developer.apple.com → Certificates → + → Developer ID Application
#    (G2 Sub-CA), upload the request, download the .cer.
# Or Xcode → Settings → Accounts → Manage Certificates → + → Developer ID
# Application, then Keychain Access → Export only that one as a .p12.
# For notarization (optional), an app-specific password made at account.apple.com
# → Sign-In and Security → App-Specific Passwords.
# For iCloud (optional, needs notarization), the Developer ID profile in
# app/src-tauri/yoru.production.provisionprofile (docs/icloud.md).

set -euo pipefail

team=79AC9KV62T
usage() { sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 1; }
[[ $# -ge 1 && -f "$1" ]] || usage
gh auth status > /dev/null

if [[ $# -eq 2 ]]; then
    # Key and .cer: bundle them, with Apple's G2 intermediate that the CI's
    # keychain lacks, into a .p12 under a random password nobody types.
    [[ -f "$2" ]] || usage
    work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
    openssl x509 -inform DER -in "$2" -out "$work/cert.pem" 2> /dev/null ||
        openssl x509 -in "$2" -out "$work/cert.pem"
    curl -fsSL https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer |
        openssl x509 -inform DER -out "$work/ca.pem"
    openssl x509 -in "$work/cert.pem" -noout -pubkey > "$work/a"
    openssl pkey -in "$1" -pubout > "$work/b"
    cmp -s "$work/a" "$work/b" || { echo "yoru: $1 is not the key of $2" >&2; exit 1; }
    p12_password="$(openssl rand -hex 24)"
    p12="$work/DeveloperID.p12"
    # 3DES and SHA-1: the p12 that macOS's `security import` reads.
    P="$p12_password" openssl pkcs12 -export -inkey "$1" -in "$work/cert.pem" \
        -certfile "$work/ca.pem" -out "$p12" -passout env:P \
        -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
else
    p12="$1"
    read -r -s -p "Password of $(basename "$p12"): " p12_password; echo
fi
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

read -r -p "Apple ID (email) for notarization, empty to skip it: " apple_id
if [[ -n "$apple_id" ]]; then
    read -r -s -p "App-specific password: " apple_password; echo
fi

base64 -i "$p12" | tr -d '\n' | gh secret set APPLE_CERTIFICATE
printf '%s' "$p12_password" | gh secret set APPLE_CERTIFICATE_PASSWORD
printf '%s' "$identity" | gh secret set APPLE_SIGNING_IDENTITY
printf '%s' "$team" | gh secret set APPLE_TEAM_ID
profile="$(dirname "$0")/../app/src-tauri/yoru.production.provisionprofile"
if [[ -f "$profile" ]]; then
    base64 -i "$profile" | tr -d '\n' | gh secret set APPLE_PROVISIONING_PROFILE
    echo "yoru: with the Developer ID profile: the notarized build has iCloud"
fi
if [[ -n "$apple_id" ]]; then
    printf '%s' "$apple_id" | gh secret set APPLE_ID
    printf '%s' "$apple_password" | gh secret set APPLE_PASSWORD
    echo "yoru: secrets set; the next push to main or develop builds a signed, notarized app"
else
    echo "yoru: secrets set; the next push builds a signed app, not notarized (run again with an Apple ID to notarize)"
fi
