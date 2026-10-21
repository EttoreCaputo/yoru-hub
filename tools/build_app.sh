#!/usr/bin/env bash
# Build the macOS app signed with your own Apple certificate, so the keychain
# knows it from one build to the next: after one "Always Allow" it stops
# asking for the login password. The release workflow signs ad hoc ("-" in
# tauri.conf.json), and an ad-hoc signature changes with every build.
#
#   tools/build_app.sh [tauri build arguments...]
#
# e.g. tools/build_app.sh --config src-tauri/tauri.sidecars.conf.json
# The first "Apple Development" or "Developer ID Application" identity in
# the keychain is used; pick another with APPLE_SIGNING_IDENTITY (a name or
# a SHA-1 hash).

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    # By hash: two certificates with the same name make codesign refuse it.
    APPLE_SIGNING_IDENTITY="$(security find-identity -v -p codesigning |
        awk '/"(Apple Development|Developer ID Application):/ {print $2; exit}')"
fi
if [[ -z "$APPLE_SIGNING_IDENTITY" ]]; then
    echo "yoru: no Apple Development or Developer ID certificate in the keychain" >&2
    exit 1
fi
export APPLE_SIGNING_IDENTITY
echo "yoru: signing with $APPLE_SIGNING_IDENTITY"

cd "$root/app"
# Not notarized, so the hardened runtime buys nothing, and its library
# validation could stop the sidecars loading their libraries.
npm run tauri build -- --config '{"bundle":{"macOS":{"hardenedRuntime":false}}}' "$@"
