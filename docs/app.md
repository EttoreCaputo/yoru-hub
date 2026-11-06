# Yoru, the desktop app

The desktop app in `app/`: Tauri 2 (`app/src-tauri/`, crate `yoru-app`) around `yoru-core`, with a Svelte 5 UI (`app/src/`). Paths below are under `app/`.

## The UI

- `src/lib/store.svelte.ts`: what the window knows, kept current by the Rust side's events (`bridge`, `inbox`, `chat`, `tasks`, `schedule`, `connectors`, `flash`, `assistant`).
- `src/lib/api.ts`: every command goes through `call()`. Outside Tauri, `src/lib/preview.ts` answers with a mocked Yoru, so `npm run dev` shows every page in a browser (`?setup`, `?board=off`, `?key=none`, `?brain=groq`).
- `src/lib/pages/`: Today, Conversation, Inbox, Connectors, Tasks (the jobs you give Yoru, with `ui/TaskEditor.svelte`), Routines (the recap, quiet hours, watchers), Memory, Meetings (recordings, `docs/meetings.md`), Spoken (what ElevenLabs said), Animations (every animation clip the board has, to play on it one at a time or all in a row, mirrored or in a service's colours; through the board's `debug/clips` and `debug/clip`, firmware 0.20.9 or later), Settings. Settings opens on a list of parts: brain, MCP server, voice, language, the board (it was the Yoru page until 2026-10-21) and about. `src/lib/Setup.svelte` is the first run.
- `src/lib/updates.ts` and `bundled.ts`: the firmware the app carries and what flashing it changes, from `firmware/yoru/`.
- `src/lib/i18n/`: `en.ts` and `it.ts`, key for key (`npm run check` fails on a missing one).

## The Rust side

- `src-tauri/src/lib.rs`: the bridge to the board, voice, firmware, WiFi, the board's animation clips (`yoru_clips`, `yoru_play_clip`, `yoru_stop_clip`: raw JSON-RPC test hooks, not MCP tools), the MCP server's entry in Claude Code's config, timers, the tray.
- `src-tauri/src/assistant.rs`: the assistant's commands (ask, inbox, tasks, memory, watchers, recap, connectors) and the scheduler and connectors started with the app.
- `src-tauri/src/meetings.rs`: the meetings' commands (sources, start, stop, the recordings, import, transcripts one at a time, exports, the meetings' settings, the model's download) and the `meeting` event. The recording runs in `yoru-audio`, the Swift helper in `../audio/`, which `src-tauri/build.rs` builds into `src-tauri/binaries/` on every macOS build (it needs Xcode or its command line tools); `src-tauri/Info.plist` says why macOS asks for the microphone and the system audio.

## Commands

In `app/`:

```bash
npm run dev           # the UI in a browser, mocked
npm run tauri dev     # the real app
npm run check         # svelte-check
npm run build         # the UI bundle
```
