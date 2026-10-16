# Companion Hub

![Rust](https://img.shields.io/badge/Rust-2024-orange)
![Tauri](https://img.shields.io/badge/Tauri-2-blue)
![License](https://img.shields.io/badge/license-MIT-green)

The program on your computer for the ESP32-S3 desk companions:

- [DualEye PC Monitor](https://github.com/EttoreCaputo/esp32s3-dualeye-pcmonitor): two round displays that show your computer's sensors, music, timers and Claude Code, with a pair of eyes and a voice assistant;
- [Yoru](https://github.com/EttoreCaputo/Yoru): a desk pet on a round AMOLED that reacts to you and your computer, and an assistant you can talk to.

It reads the computer's sensors and sends them to the boards, runs the voice assistant (speech-to-text, a language model and text-to-speech, local or in the cloud), flashes and updates the firmware, pairs the boards over WiFi, and lets Claude and other MCP clients use them. It is meant to drive several boards at once, of either kind: see [docs/multiboard.md](docs/multiboard.md) for where that stands.

Status: early development (0.1.0). The code comes from DualEye's `host/` (app 2.0.4); today it still drives one board at a time and knows DualEye best.

## Repository layout

```
core/            library: link (protocol v2 over USB and WiFi), sensors, snapshot, bridge and hub,
                 voice (STT, language model agent, TTS), MCP server, Claude Code, timers, flasher
cli/             `companion` command line
app/             desktop app: Svelte UI in src/, Tauri shell in src-tauri/
power-helper/    macOS root helper that reads the CPU power
firmware/        each board's firmware image, embedded in the app (fetched, not committed)
tools/           firmware fetch, voice sidecars and power helper builds
docs/            design notes and licenses
```

## Firmware

The app carries each board's firmware and offers to flash it when a board runs an older one. It flashes a board only with its own kind's image; for a blank board, which can't say what it is, you pick the kind. The images come from the firmware repositories, which are expected next to this one:

```
ESP-IDF/
  companion-hub/
  esp32s3-dualeye-pcmonitor/
  yoru/
```

Build each firmware (`idf.py build merge-bin` in its repository), then copy the images here:

```bash
tools/fetch_firmware.sh
```

It fills `firmware/dualeye/` and `firmware/yoru/` with `merged-binary.bin`, `version.txt` and `FIRMWARE_CHANGELOG.md`. `DUALEYE_REPO` and `YORU_REPO` point it at other checkouts. Run it again after a firmware change and rebuild the app.

## Build

| Part | Needs |
|------|-------|
| CLI | [Rust](https://rustup.rs) ≥ 1.85 (edition 2024) |
| Desktop app | Rust, [Node.js](https://nodejs.org) ≥ 20.19, and [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS |

CLI:

```bash
cargo build --release       # target/release/companion
```

Plain `cargo` commands only touch `core` and `cli` (the workspace's default members), so they work without the GUI packages.

Desktop app:

```bash
cd app
npm install
npm run dev                 # UI only, in a browser, with synthetic data: no board or Rust needed
npm run tauri dev           # the real app, hot reload on UI changes
npm run tauri build         # release build + installers, in target/release/bundle/
```

On macOS, `tauri dev` and `tauri build` first build the power helper (`tools/build_power_helper.sh`). The voice sidecars (whisper.cpp's and llama.cpp's servers) are built by `tools/build_sidecars.sh` and bundled with `--config src-tauri/tauri.sidecars.conf.json` (`tauri.sidecars.macos.conf.json` on macOS).

## Checks

```bash
cargo test --workspace --exclude companion-hub
```

```bash
cd app && npm run check
```

## Releases

Every push to `main` runs [`.github/workflows/release.yml`](.github/workflows/release.yml): it checks out and builds both firmware repositories with ESP-IDF 6.1, then builds the app on Linux, Windows and macOS with those images and the voice helpers inside, and uploads everything, plus the bare firmware images, to the GitHub release `v<version>` from `app/src-tauri/tauri.conf.json`. A push to `develop` does the same for the pre-release `v<version>-develop`.

## Protocol

The boards speak protocol v2: [DualEye's protocol](https://github.com/EttoreCaputo/esp32s3-dualeye-pcmonitor/blob/main/docs/protocol.md), and [what differs on Yoru](https://github.com/EttoreCaputo/Yoru/blob/main/docs/protocol.md).

## License

MIT, see [LICENSE](LICENSE). Third-party components and models: [docs/licenses.md](docs/licenses.md).
