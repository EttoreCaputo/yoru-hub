#!/usr/bin/env bash
# Finish the public build with iCloud: a Yoru.app built with
# tauri.icloud.production.conf.json (the Developer ID profile embedded) is
# signed again, notarized and put in a notarized .dmg (docs/icloud.md).
#
#   tools/sign_release.sh <Yoru.app> <out.dmg>
#
# Needs APPLE_SIGNING_IDENTITY (the Developer ID Application identity, in a
# keychain codesign can use). With APPLE_ID, APPLE_PASSWORD (an app-specific
# password) and APPLE_TEAM_ID the app and the .dmg are notarized and stapled;
# without them they are only signed, enough to try the build on this Mac.
#
# Why not let Tauri do it: it signs the sidecars with the app's entitlements,
# and iCloud and push are restricted ones that a bare helper can't carry (no
# profile grants them to it), so macOS kills it. They get the plain ones back
# here, before the app is sealed and notarized.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
app="${1:?usage: sign_release.sh <Yoru.app> <out.dmg>}"
dmg="${2:?usage: sign_release.sh <Yoru.app> <out.dmg>}"
identity="${APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY is not set}"
plain="$root/app/src-tauri/Entitlements.plist"
icloud="$root/app/src-tauri/Entitlements.icloud.production.plist"

sign() { codesign --force --options runtime --timestamp -s "$identity" "$@"; }

notarize() {
    [[ -n "${APPLE_ID:-}" ]] || return 0
    xcrun notarytool submit "$1" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
        --team-id "${APPLE_TEAM_ID:-79AC9KV62T}" --wait --timeout 30m
}

[[ -f "$app/Contents/embedded.provisionprofile" ]] || {
    echo "yoru: $app has no embedded.provisionprofile" >&2
    exit 1
}

for bin in "$app"/Contents/MacOS/*; do
    [[ "$(basename "$bin")" == yoru-app ]] && continue
    sign --entitlements "$plain" "$bin"
done
sign --entitlements "$icloud" "$app"
codesign --verify --strict --deep "$app"
echo "yoru: signed $app"

# The app first, so its own ticket is stapled before it goes in the .dmg:
# copied out of it, it opens offline too.
if [[ -n "${APPLE_ID:-}" ]]; then
    zip="$(mktemp -d)/Yoru.zip"
    ditto -c -k --keepParent "$app" "$zip"
    notarize "$zip"
    xcrun stapler staple "$app"
fi

stage="$(mktemp -d)"
ditto "$app" "$stage/Yoru.app"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -volname Yoru -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg" > /dev/null
codesign --force --timestamp -s "$identity" "$dmg"
if [[ -n "${APPLE_ID:-}" ]]; then
    notarize "$dmg"
    xcrun stapler staple "$dmg"
    echo "yoru: $dmg signed, notarized and stapled"
else
    echo "yoru: $dmg signed, not notarized (no APPLE_ID)"
fi
