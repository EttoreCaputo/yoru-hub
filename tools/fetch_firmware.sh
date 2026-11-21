#!/usr/bin/env bash
# Copy Yoru's firmware into firmware/yoru/, where the app embeds it: the
# merged image, its version.txt, its changelog (docs/FIRMWARE_CHANGELOG.md) and its
# animation clips' frames (tools/clips/frames.json).
#
#   tools/fetch_firmware.sh
#
# The firmware repository is expected next to this one; point elsewhere with
# YORU_REPO. Build it first (`idf.py build merge-bin`).

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
parent="$(dirname "$root")"
repo="${YORU_REPO:-$parent/yoru}"
out="$root/firmware/yoru"

if [[ ! -f "$repo/build/merged-binary.bin" ]]; then
    echo "yoru: no $repo/build/merged-binary.bin (build the firmware first)" >&2
    exit 1
fi
mkdir -p "$out"
cp "$repo/build/merged-binary.bin" "$out/merged-binary.bin"
cp "$repo/version.txt" "$out/version.txt"
cp "$repo/docs/FIRMWARE_CHANGELOG.md" "$out/FIRMWARE_CHANGELOG.md"
# The animation clips' frames, for the previews on the Animations page.
cp "$repo/tools/clips/frames.json" "$out/clips.json"
echo "yoru: firmware $(tr -d '[:space:]' < "$out/version.txt") from $repo"
