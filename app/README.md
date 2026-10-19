# Yoru, the desktop app

Tauri 2 (`src-tauri/`, crate `yoru-app`) around `yoru-core`, with a Svelte 5 UI (`src/`).

## The UI

- `src/lib/store.svelte.ts`: what the window knows, kept current by the Rust side's events (`bridge`, `inbox`, `chat`, `schedule`, `connectors`, `flash`, `assistant`).
- `src/lib/api.ts`: every command goes through `call()`. Outside Tauri, `src/lib/preview.ts` answers with a mocked Yoru, so `npm run dev` shows every page in a browser (`?setup`, `?board=off`, `?key=none`, `?brain=groq`).
- `src/lib/pages/`: Today, Conversation, Inbox, Connectors, Routines, Memory, Yoru (the board), Settings. `src/lib/Setup.svelte` is the first run.
- `src/lib/i18n/`: `en.ts` and `it.ts`, key for key (`npm run check` fails on a missing one).

## The Rust side

- `src-tauri/src/lib.rs`: the bridge to the board, voice, firmware, WiFi, Claude Code, timers, the tray.
- `src-tauri/src/assistant.rs`: the assistant's commands (ask, inbox, memory, watchers, recap, connectors) and the scheduler and connectors started with the app.

## Commands

```bash
npm run dev           # the UI in a browser, mocked
npm run tauri dev     # the real app
npm run check         # svelte-check
npm run build         # the UI bundle
```
