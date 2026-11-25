#!/usr/bin/env bash
# Build the macOS app signed with your own Apple certificate, so the keychain
# knows it from one build to the next: after one "Always Allow" it stops
# asking for the login password. The release workflow signs with the
# Developer ID certificate when its secrets are set, ad hoc otherwise.
#
#   tools/build_app.sh [tauri build arguments...]
#
# e.g. tools/build_app.sh --config src-tauri/tauri.sidecars.conf.json
# The first "Apple Development" identity in the keychain is used, since the
# iCloud profile (docs/icloud.md) is a development one; a "Developer ID
# Application" one only when there's none. Pick another with
# APPLE_SIGNING_IDENTITY (a name or a SHA-1 hash).

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    # By hash: two certificates with the same name make codesign refuse it.
    identities="$(security find-identity -v -p codesigning)"
    APPLE_SIGNING_IDENTITY="$(awk '/"Apple Development:/ {print $2; exit}' <<< "$identities")"
    [[ -n "$APPLE_SIGNING_IDENTITY" ]] ||
        APPLE_SIGNING_IDENTITY="$(awk '/"Developer ID Application:/ {print $2; exit}' <<< "$identities")"
fi
if [[ -z "$APPLE_SIGNING_IDENTITY" ]]; then
    echo "yoru: no Apple Development or Developer ID certificate in the keychain" >&2
    exit 1
fi
export APPLE_SIGNING_IDENTITY
echo "yoru: signing with $APPLE_SIGNING_IDENTITY"

cd "$root/app"
# With Xcode's iCloud profile in place (docs/icloud.md, Signing), embed it and
# claim CloudKit and push; without it the app is built as before.
icloud=()
if [[ -f "$root/app/src-tauri/yoru.provisionprofile" ]]; then
    icloud=(--config src-tauri/tauri.icloud.conf.json)
    echo "yoru: with iCloud (yoru.provisionprofile)"
fi
# Keep the bundle's Hardened Runtime and permission entitlements.
npm run tauri build -- ${icloud[@]+"${icloud[@]}"} "$@"

# Tauri signs the sidecars with the app's entitlements. iCloud and push are
# restricted: a bare helper can't carry the profile that grants them, so it
# gets the plain ones back, and the app is sealed again around it.
if (( ${#icloud[@]} )); then
    app="$root/target/release/bundle/macos/Yoru.app"
    for bin in "$app"/Contents/MacOS/*; do
        [[ "$(basename "$bin")" == yoru-app ]] && continue
        codesign --force --options runtime --timestamp=none -s "$APPLE_SIGNING_IDENTITY" \
            --entitlements "$root/app/src-tauri/Entitlements.plist" "$bin"
    done
    codesign --force --options runtime --timestamp=none -s "$APPLE_SIGNING_IDENTITY" \
        --entitlements "$root/app/src-tauri/Entitlements.icloud.plist" "$app"
    codesign --verify --strict "$app"
    echo "yoru: sidecars re-signed without iCloud (the .dmg, if built, is not)"
fi
