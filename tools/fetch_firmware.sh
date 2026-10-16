#!/usr/bin/env bash
# Copy each board's firmware into firmware/<board>/, where the app embeds it:
# the merged image, its version.txt and its FIRMWARE_CHANGELOG.md.
#
#   tools/fetch_firmware.sh
#
# The firmware repositories are expected next to this one; point elsewhere with
# DUALEYE_REPO and YORU_REPO. Build each one first (`idf.py build merge-bin`).

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
parent="$(dirname "$root")"

fetch() {
    local board="$1" repo="$2"
    local out="$root/firmware/$board"
    if [[ ! -f "$repo/build/merged-binary.bin" ]]; then
        echo "$board: no $repo/build/merged-binary.bin (build the firmware first)" >&2
        return 1
    fi
    mkdir -p "$out"
    cp "$repo/build/merged-binary.bin" "$out/merged-binary.bin"
    cp "$repo/version.txt" "$out/version.txt"
    cp "$repo/FIRMWARE_CHANGELOG.md" "$out/FIRMWARE_CHANGELOG.md"
    echo "$board: firmware $(tr -d '[:space:]' < "$out/version.txt") from $repo"
}

fetch dualeye "${DUALEYE_REPO:-$parent/esp32s3-dualeye-pcmonitor}"
fetch yoru "${YORU_REPO:-$parent/yoru}"
