# Yoru

![Rust](https://img.shields.io/badge/Rust-2024-orange)
![Tauri](https://img.shields.io/badge/Tauri-2-blue)
![License](https://img.shields.io/badge/license-MIT-green)

Yoru is a personal assistant that lives on your computer and has a body: [a round ESP32-S3 board](https://github.com/EttoreCaputo/Yoru) on the desk, a pearl sphere on true black. It connects to the services you use (mail, calendar, issues, documents…) through MCP connectors, keeps an eye on them, tells you when something is worth knowing, and answers when you ask, typed in the app or said to the board.

This repository is its host program: the Rust core, the `yoru` command line and the desktop app. The plan and where it stands: [assistant.md](assistant.md); recording and summarizing meetings, planned: [meetings.md](meetings.md); the memory as a graph, with voices: [memory.md](memory.md); one agent and a second brain, planned: [agent.md](agent.md), and the research behind it: [memory-research.md](memory-research.md); syncing between Macs through iCloud (Settings → iCloud, since 2026-11-25): [icloud.md](icloud.md); the app's code: [app.md](app.md).

Status: early development (0.1.0).

Platform: **macOS only** for now (Apple Silicon and Intel). Some code still builds elsewhere, but Linux and Windows are neither built, released nor tested.

## What it does

- **Answers**: with a brain you pick: a model on [Groq](https://console.groq.com/keys) (free, with daily limits: a good start), GPT (OpenAI's API, paid, cheap), Gemini ([Google AI Studio](https://aistudio.google.com/apikey): a free tier with daily limits, or paid), a model on this computer through [Ollama](https://ollama.com) (free and private) or Claude (Anthropic's API, paid, the best, with web search), with your connectors' tools, the board's tools and the timers. Anything that changes something (sending a mail, creating an issue) asks first, on the board and in the app.
- **Talks live, if you like**: with Gemini Live (free tier, a Google key) Yoru hears, thinks and speaks in one model, answering in about a second, with Google's voices and without the connectors' tools.
- **Watches**: watchers check a connector's tool or do a task every so often, and only what's new and matters becomes a notification. Scheduled tasks, the morning recap among them, quiet hours and do-not-disturb.
- **Does what you give it to do**: tasks you assign Yoru in words, once or on a schedule (every day or some weekdays, every month, every few hours), done on his own when it's time; each one reads and tells only, asks before acting, or acts without asking, as you choose. The board's Tasks app shows them, with "Do it now".
- **Works your coding agents**: tell him to continue a conversation you had with Claude Code, Codex or OpenCode, or to start a new one in one of your projects or in a fresh folder, and he sends your words to the agent, which works on its own; its answer comes to the inbox when it's done. Today's page and the notch show which agents are working right now, his and yours.
- **Keeps time**: timers, a pomodoro and reminders, shown and handled in the board's Timers app.
- **Remembers**: a graph of the people and things in your life and the facts about them (`memory.db`), searched for each answer, learnt from conversations and meetings once they're over, edited in the app. It knows who is speaking by their voice.
- **Talks**: after the wake word or with PWR held on the board, it transcribes on this computer (whisper.cpp) or with your provider's Whisper, and speaks with Piper on this computer or with ElevenLabs or Groq. It keeps listening after an answer, tells a goodbye from a new request (EmbeddingGemma) and hears when you finished the sentence (Smart Turn); the Spoken page keeps what ElevenLabs and Gemini Live said, each apart.
- **Lives in your notch too**: if you like, a small Yoru around the Mac's camera notch, who opens into a panel with the last exchange and the cards waiting, and listens while you hold ⌥ Space, through the Mac's microphone and speakers ([notch.md](notch.md)).
- **Records meetings**: what the computer plays (everything, or one app such as a call) and your microphones, each as a track of its own, an audio or video file you bring in, or the recordings of a Pocket recorder, copied over Bluetooth ([pocket.md](pocket.md)); transcribed on this computer with Parakeet, who speaks told apart with Nemotron (up to 8 people per track, each nameable), saved as Markdown, SRT or text, and summarized (points, decisions, things to do, open questions) by the local model or the brain ([meetings.md](meetings.md)).
- **Has a body**: the board shows notifications, timers, tasks and Allow/Deny cards, goes on an errand while a connector is called, listens after its wake word, and speaks. Any MCP client (Claude Code, Claude Desktop…) can make it react, speak, notify you and set timers, through its MCP server.

## Repository layout

```
core/            library: the assistant (inbox, memory, tasks, watchers), connectors (MCP client,
                 OAuth, keychain), link (protocol v2 over USB and WiFi), bridge and hub, voice
                 (STT, TTS), MCP server, timers, flasher
cli/             `yoru` command line
app/             desktop app: Svelte UI in src/, Tauri shell in src-tauri/
audio/           yoru-audio, the Swift helper that records meetings and is the notch's ears and voice
                 (built by the app's build.rs)
firmware/        the board's firmware image, embedded in the app (fetched, not committed)
tools/           firmware fetch, the signed Mac build and voice sidecar builds
docs/            this README, the plans, the app's code, OAuth clients, licenses, privacy and terms
```

## Firmware

The app carries Yoru's firmware and offers to flash it when the board runs an older one: with esptool when the board is on USB, over its link when it's on WiFi (`core/src/ota.rs`). Only the app's part of the merged image goes over WiFi, about a minute for 3 MB, and the board goes back to its firmware before if the new one doesn't get back on the WiFi. `cargo run -p yoru-core --example ota -- ../yoru/build/yoru.bin [BOARD_ID]` does the same from the terminal. The image comes from the firmware repository, expected next to this one (`../yoru`). Build it there (`idf.py build merge-bin`), then:

```bash
tools/fetch_firmware.sh     # fills firmware/yoru/; YORU_REPO points elsewhere
```

`firmware/yoru/` then holds `merged-binary.bin`, `version.txt` and `FIRMWARE_CHANGELOG.md` (from the firmware's `docs/`). The app embeds them at compile time; they are not committed, and the release workflow builds them in CI.

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
                            # (?setup opens the first run, ?mic=unknown or denied the permissions, ?board=off, ?key=none and ?brain=groq other states)
npm run tauri dev           # the real app, hot reload on UI changes
npm run tauri build         # release build + installers, in target/release/bundle/
```

On a Mac, build with `tools/build_app.sh` instead (same arguments): it signs the app with your Apple Development certificate. The keychain then recognises it from one build to the next and stops asking for the login password after one "Always Allow"; a build signed ad hoc is a new app to it every time.

The voice sidecars (whisper.cpp's and llama.cpp's servers) are built by `tools/build_sidecars.sh` and bundled with `--config src-tauri/tauri.sidecars.conf.json`.

## Checks

```bash
cargo check --workspace && cargo test --workspace
cd app && npm run check && npm run build
```

## Releases

Every push to `main` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml): it builds Yoru's firmware with ESP-IDF 6.1, then the app for macOS (Apple Silicon and Intel; the only platform Yoru supports) with that image and the voice helpers inside, and uploads everything, plus the bare firmware image, to the GitHub release `v<version>` from `app/src-tauri/tauri.conf.json`. A push to `develop` does the same for the pre-release `v<version>-develop`.

The release signs the app with the Developer ID Application certificate of team `79AC9KV62T` and has Apple notarize it, so macOS opens the downloaded app without asking. The workflow reads it from the repository secrets `APPLE_CERTIFICATE` (the .p12 in base64), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, and for notarization `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`. `tools/set_signing_secrets.sh <DeveloperID.p12>` sets them all; its header says how to make the certificate and the password. Without them the app is signed ad hoc and not notarized, and the release notes say how to open it anyway. A release build has no iCloud: the profile is a development one, for this Mac's `tools/build_app.sh` builds (see [icloud.md](icloud.md)).

## Protocol

The board speaks protocol v2: [Yoru's protocol](https://github.com/EttoreCaputo/Yoru/blob/main/docs/protocol.md).

## License

MIT, see [LICENSE](../LICENSE). Third-party components and models: [licenses.md](licenses.md). [Privacy](privacy.md) and [terms](terms.md).
