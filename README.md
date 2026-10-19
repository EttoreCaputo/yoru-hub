# Yoru

![Rust](https://img.shields.io/badge/Rust-2024-orange)
![Tauri](https://img.shields.io/badge/Tauri-2-blue)
![License](https://img.shields.io/badge/license-MIT-green)

Yoru is a personal assistant that lives on your computer and has a body: [a round ESP32-S3 board](https://github.com/EttoreCaputo/Yoru) on the desk, a pearl sphere on true black. It connects to the services you use (mail, calendar, issues, documents…) through MCP connectors, keeps an eye on them, tells you when something is worth knowing, and answers when you ask, typed in the app or said to the board.

This repository is its host program: the Rust core, the `yoru` command line and the desktop app. The plan is [docs/assistant.md](docs/assistant.md).

Status: early development (0.1.0).

## What it does

- **Answers**: with a brain you pick: a model on [Groq](https://console.groq.com/keys) (free, with daily limits: a good start) or Claude (Anthropic's API, paid, the best, with web search), with your connectors' tools, the board's tools and the timers. Anything that changes something (sending a mail, creating an issue) asks first, on the board and in the app.
- **Watches**: watchers check a connector's tool or do a task every so often, and only what's new and matters becomes a notification. A morning recap, quiet hours and do-not-disturb.
- **Remembers**: `memory.md` in the data folder, read every turn, edited in the app.
- **Has a body**: the board shows notifications and Allow/Deny cards, listens after its wake word, and speaks. Claude Code's sessions and permission requests reach it too.

## Repository layout

```
core/            library: the assistant (inbox, memory, routines, watchers), connectors (MCP client,
                 OAuth, keychain), link (protocol v2 over USB and WiFi), bridge and hub, voice
                 (STT, TTS), MCP server, Claude Code, timers, flasher
cli/             `yoru` command line
app/             desktop app: Svelte UI in src/, Tauri shell in src-tauri/
firmware/        the board's firmware image, embedded in the app (fetched, not committed)
tools/           firmware fetch and voice sidecar builds
docs/            design notes and licenses
```

## Firmware

The app carries Yoru's firmware and offers to flash it when the board runs an older one. The image comes from the firmware repository, expected next to this one (`../yoru`). Build it there (`idf.py build merge-bin`), then:

```bash
tools/fetch_firmware.sh     # fills firmware/yoru/; YORU_REPO points elsewhere
```

## Build

| Part | Needs |
|------|-------|
| CLI | [Rust](https://rustup.rs) ≥ 1.85 (edition 2024) |
| Desktop app | Rust, [Node.js](https://nodejs.org) ≥ 20.19, and [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS |

```bash
cargo build --release       # target/release/yoru
```

Plain `cargo` commands only touch `core` and `cli` (the workspace's default members), so they work without the GUI packages.

```bash
cd app
npm install
npm run dev                 # UI only, in a browser, with a mocked Yoru: no board or Rust needed
                            # (?setup opens the first run, ?board=off, ?key=none and ?brain=groq other states)
npm run tauri dev           # the real app, hot reload on UI changes
npm run tauri build         # release build + installers, in target/release/bundle/
```

The voice sidecars (whisper.cpp's and llama.cpp's servers) are built by `tools/build_sidecars.sh` and bundled with `--config src-tauri/tauri.sidecars.conf.json`.

## Checks

```bash
cargo check --workspace && cargo test --workspace
cd app && npm run check && npm run build
```

## Releases

Every push to `main` runs [`.github/workflows/release.yml`](.github/workflows/release.yml): it builds Yoru's firmware with ESP-IDF 6.1, then the app on Linux, Windows and macOS with that image and the voice helpers inside, and uploads everything, plus the bare firmware image, to the GitHub release `v<version>` from `app/src-tauri/tauri.conf.json`. A push to `develop` does the same for the pre-release `v<version>-develop`.

## Protocol

The board speaks protocol v2: [Yoru's protocol](https://github.com/EttoreCaputo/Yoru/blob/main/docs/protocol.md).

## License

MIT, see [LICENSE](LICENSE). Third-party components and models: [docs/licenses.md](docs/licenses.md).
