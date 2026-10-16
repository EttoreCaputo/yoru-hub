# Companion Hub

The host program (Rust core, CLI, Tauri + Svelte app) for the ESP32-S3 desk companions DualEye (`../esp32s3-dualeye-pcmonitor`) and Yoru (`../yoru`). It came out of DualEye's `host/`. Multi-board plan: `docs/multiboard.md`.

## Rule: the user makes the commits

Never run `git commit`, `git push` or anything else that writes history. Leave the changes in the working tree; the user commits them.

## Rule: everything in the code is in English

Identifiers, comments, log messages, strings, file names and docs are written in English, even when the conversation is in another language. The only exception is text the user sees in another language on purpose (the app's Italian translations).

## Rule: board differences go through the profile

Never hardcode one board's names (mDNS service, crypto labels, `hello.board`, tools, faces) in shared code: they belong to that board's profile. A change to the protocol is made in both firmware repositories too (`docs/protocol.md` in each).

## Checks after a change

- `cargo check --workspace` and `cargo test --workspace --exclude companion-hub`
- `npm run check` in `app/`
- The app needs `firmware/<board>/` filled by `tools/fetch_firmware.sh` and, on macOS, the power helper from `tools/build_power_helper.sh`.

## Versions

The app version is in `app/src-tauri/tauri.conf.json`, `app/package.json`, and every crate's `Cargo.toml` (with the `version` on the app's `companion-core` dependency). Keep them equal; CI names the release after the Tauri one.

## Conventions

Match the existing style: short functions, comments that say why.
