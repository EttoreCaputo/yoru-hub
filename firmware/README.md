# Firmware images

One folder per board, each with `merged-binary.bin`, `version.txt` and `FIRMWARE_CHANGELOG.md`. The app embeds them at compile time. They are not committed: `tools/fetch_firmware.sh` copies them from the firmware repositories (locally) and the release workflow builds them (in CI).

- `dualeye/`: [esp32s3-dualeye-pcmonitor](https://github.com/EttoreCaputo/esp32s3-dualeye-pcmonitor)
- `yoru/`: [Yoru](https://github.com/EttoreCaputo/Yoru)
