# Yoru Hub

The host program of Yoru (`../yoru`): Rust core, CLI, Tauri + Svelte app. The folder is `yoru-hub`; it was Companion Hub until 2026-10-19. It came out of DualEye's `host/`.

Since 2026-10-19 it is Yoru's personal assistant, for Yoru only: MCP connectors with OAuth (`core/src/connectors/`), the assistant on Claude (`core/src/assistant/`, `core/src/cloud/anthropic.rs`), watchers, notifications, memory, routines. The plan is `docs/assistant.md`. DualEye is gone from here (it keeps its own `host/`); `docs/multiboard.md` is history.

## Rule: the user makes the commits

Never run `git commit`, `git push` or anything else that writes history. Leave the changes in the working tree; the user commits them.

## Rule: everything in the code is in English

Identifiers, comments, log messages, strings, file names and docs are written in English, even when the conversation is in another language. The only exception is text the user sees in another language on purpose (the app's Italian translations).

## Rule: protocol changes go to the firmware too

A change to the board protocol is made in `../yoru/docs/protocol.md` as well.

## Checks after a change

- `cargo check --workspace` and `cargo test -p yoru-core --features mcp,provision,connectors` (the crates are `yoru-core`, `yoru-cli`, `yoru-app`)
- `npm run check` in `app/`
- The app needs `firmware/yoru/` filled by `tools/fetch_firmware.sh`.

## Versions

The app version is in `app/src-tauri/tauri.conf.json`, `app/package.json`, and every crate's `Cargo.toml` (with the `version` on the app's `yoru-core` dependency). Keep them equal; CI names the release after the Tauri one.

## Conventions

Match the existing style: short functions, comments that say why.
