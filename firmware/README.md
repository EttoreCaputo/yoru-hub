# Firmware image

`yoru/` holds Yoru's `merged-binary.bin`, `version.txt` and `FIRMWARE_CHANGELOG.md`. The app embeds them at compile time. They are not committed: `tools/fetch_firmware.sh` copies them from [Yoru's repository](https://github.com/EttoreCaputo/Yoru) (locally) and the release workflow builds them (in CI).
