# Several boards at once

Goal: one Companion Hub drives any number of boards together, of either kind. For example, a DualEye on USB and a Yoru on WiFi, or two Yorus. Services that belong to the computer run once and are shared. Everything that belongs to a board is kept per board, keyed by the `id` in its `hello`.

Status: all ten steps are done. The core, the CLI and the app drive several boards at once (`Manager`); the window shows one at a time, picked in the title bar. None of it has been tried with two boards at once in the app yet; what's still open is under "Left over" at the end.

- Step 1: the repository, the names and the firmware folder.
- Step 2: `core/src/profile.rs`, wired into the parts below:
  - **WiFi.** Each kind uses its own service and handshake labels, and a paired board keeps its `kind`.
  - **Legacy version line.**
  - **Tools kept on disk.** One `board-tools-<kind>.json` per kind.
  - **Agent and quip prompts.** The board's body comes from its profile (`Toolbox::kind`).
  - **MCP.** The instructions come from the kind, and the tools are kept per kind.
  - **Bridge.** It reads the kind from `hello`, and leaves faces and rotation alone on a board without them.
- Step 3: both firmwares are embedded.
  - **App.** `firmware_for(kind)` gives each image. `firmware::check_image` refuses an image whose `project` isn't the kind's.
  - **`flash_board`.** It takes a `kind` and refuses it when the connected board says it is another kind.
  - **`BoardFirmware::Version`.** It carries the board's `kind`.
  - **Frontend.** It reads `firmware/*/` with `import.meta.glob` (`app/src/lib/bundled.ts`), and offers updates from the board's own changelog. A board that doesn't say what it is (a blank one) gets a choice of firmware in the device tab and in the setup.

- Step 4: `bridge.rs` is split into `core/src/bridge/`:
  - **`sampler.rs`.** The computer's side, once for every board: sensors, Claude usage, timers, music, presence, the pet's diary and quips. It publishes each snapshot to `Latest`, and samples only while a session is open, as before.
  - **`session.rs`.** One board's connection: link, handshake, voice pipeline, gaze thread. It sends each published snapshot after its behaviour tailors it.
  - **`behaviour.rs`, `dualeye.rs`, `yoru.rs`.** `BoardBehaviour`, picked from `hello`. DualEye keeps its faces, rotation and cover. Yoru gets no faces or cover, and its eyes always get the pointer.
  - **`alerts.rs`.** The thread for Claude alerts, timers and quips. The scene before a quip is the profile's `scene_tool` (`play_eyes` or `react`).

  It was tried on both boards over USB: each one was recognised and streamed.
- Step 5: `core/src/bridge/manager.rs`.
  - **`Manager::spawn(config, max_boards, sink)`.** It finds every Espressif port on USB and every paired WiFi board, and runs one session per target on its own thread. A session that ends is retried with a growing pause.
  - **USB first.** A WiFi session ends when the same board id is ready on USB. WiFi waits while USB boards are still saying who they are. At the cap, WiFi gives way to a board plugged in on USB.
  - **Events.** They arrive as `ManagerEvent { target, board, board_kind, ..BridgeEvent }`; the host's own events have no target.
  - **Alerts.** They go to a `Targets` registry, and the board connected first speaks.
  - **Hub.** It keeps one attached board (`attach_if_free`, `detach`), handed to another session when that board leaves.
  - **`Bridge` and `bridge::run`.** Both are the manager with `max_boards` 1. With two boards plugged in, they now drive the first one instead of finding none.
  - **`cargo run -p companion-core --example boards`.** It prints what every board does. Tried with a Yoru and a DualEye on USB at once: both were recognised and streamed from one sampler.
  - **Not yet.** There are no separate `BoardAdded`/`BoardRemoved` events, since `Connected`, `Firmware` and `Disconnected` carry the board. The DualEye faces are still shared between DualEyes. The WiFi path hasn't been tried with the manager.
- Step 6: `core/src/bridge/floor.rs` and the speaker choice in `alerts.rs`.
  - **Voice floor.** A board's `utterance_start` claims the floor (`Receiving::gated`). While another board holds it, the utterance isn't captured: the board gets `voice/stop` and `voice/state idle`. Its own follow-ups and barge-ins claim it again. The pipeline releases it when the conversation ends, and the manager when the session ends. It lapses 30 s after its holder last started listening. Both firmwares already had `voice/stop`.
  - **Who speaks.** `Targets::speaker(origin, primary)`: the board a timer was set from (`Timer::board`, set by `Timers::call_tool_from` and passed on to the next pomodoro phase), then `BridgeConfig::primary` (a board id; the app sets it in step 8), then the board of the latest conversation (`Floor::last`), then the first connected. Quips wait while any board is in a conversation.
  - **Sidecars.** Not serialised: Whisper, llama.cpp and Piper are HTTP servers that queue concurrent requests, and the floor already keeps conversations one at a time.
  - **Not yet.** Tried only in tests, not with two boards hearing one wake word.
- Step 7: the hub, MCP and CLI.
  - **Hub.** Every ready session attaches its board (`Hub::attach`), each with its own request lock, settings flag and `host/say`. A request's params take `board` (an id, or a kind when only one of that kind is attached). Without it the request goes to the only board, then the only board with the tool, then the primary one; otherwise error `-32002` (`SEVERAL_BOARDS`). `tools/list` without `board` is the union. New methods `host/boards` (`[BoardInfo]`) and `host/hello`. `hello` adds `boards` and keeps `board` and `port` for old clients.
  - **`Board` client.** `Board::on(selector)`, `list_tools_on`, `call_tool_on`, `say_on`, `boards()` and `attached()`. Over the port, a selector that doesn't match the board there is refused.
  - **MCP.** With several boards, the board tools are the union and each tool (and `speak`) gets an optional `board` enum (ids and kinds). `list_boards` is new. The instructions describe every connected kind and say to pass `board`. Each board's tools are kept under its kind.
  - **CLI.** `--board <id|kind>` (or `COMPANION_BOARD`), written after the subcommand (`companion call --board yoru react …`), since top-level options conflict with subcommands. `companion boards [--json]`. Streaming runs the Manager with no cap. `--port` pins one session, and lines get a `[board]` tag once a second board shows up. `wifi pair` already kept the kind (step 2).
  - **Tried.** `companion boards` with a Yoru on USB, through the hub of the installed app (an older one without `host/boards`). Not yet with two boards behind the new hub.
- Step 8: the app's Rust side (`app/src-tauri/src/lib.rs`).
  - **Manager.** The app runs `Manager` with no cap (a port in the settings pins one). `Devices` keeps a `Link` per session target, plus the host's (waiting, alerts, quips).
  - **Events.** Every `ManagerEvent` goes out as `manager`. As `bridge` go the host's own and those of the current board (the primary one, else the first connected), so today's frontend keeps showing one board. `boards` (`[DeviceInfo]`) goes out when a board connects, says its firmware, leaves, or the primary changes.
  - **Commands.** Every command about a board takes an optional `board` (an id, a kind or a session's target), and without it reaches the current board. `status` takes `board` and adds `board`, `board_kind`, `boards` and `primary`. New: `boards`, `set_primary`, `yoru_settings`, `yoru_set_setting`, `yoru_react` and `yoru_show_menu`.
  - **Settings.** `boards: {id: {faces, rotation}}` and `primary`. The old `faces` and `rotation` are what a DualEye without its own gets. In core, `BridgeConfig::board_settings` and `screens(id)` give each DualEye its own faces, so they are no longer shared.
  - **Flashing.** `Manager::pause(port)` ends that port's session and starts none there until `resume`; the other boards carry on. A session that ends on purpose (paused, or its board moved to USB) now raises `Disconnected` with why.
  - **Tray.** The tooltip says "Companion Hub" and how many boards there are when there are several.
  - **Not yet.** The pointer setting is still shared. Not tried in the running app: the installed app holds the port, and a dev build would take over from it.
- Step 9: the frontend.
  - **One store that follows the board shown**, instead of a `Device` store per board. `show_board` (Rust) picks the board the window shows: the `bridge` events and the commands without `board` follow it. On a switch `monitor` reads that board's `status` again (logs, transcripts, firmware, faces). It's far smaller than splitting the 1500-line store, and a board not shown keeps its record in the app, ready when it's picked. A `Device` per board remains possible on top of `manager` events if the window should ever show two at once.
  - **Switcher.** With several boards, the title bar has a chip per board (kind, the end of its id, ★ for the primary one). The name in the bar is the board's kind, or "Companion Hub" with several.
  - **`YoruBoard.svelte`.** The Yoru mirror from `yoru/docs/CONCEPT.md`: a matte sphere in the black, two dark pill eyes that follow the pointer and blink, the state tint and halo from the voice state, and a dim dot with z's while there's no board. The glass/pixels toggle and the face picker are DualEye only.
  - **Device tab.** "Boards", with the primary picker, when there are several. "Yoru", for a Yoru, built from `get_settings`: 0/1 settings as switches, other ranges as sliders, each sent with `set_setting`, plus a button that opens the menu on the board.
  - **Commands.** `monitor` passes `board: boardId` to every board command.
  - **Voice and Pet tabs on a Yoru.** A Yoru has `set_volume`, `set_mic` (`wake_sound`), `set_pet`, `get_pet` and `play_sound` as DualEye does, so the volume, the wake sound, the pet's switches, its mood card and the sounds stay as they are. It has no `set_eyes` and no `play_eyes`: the eyes/ring switch (Voice) and the idle eyes (Pet) are hidden, and so are their search entries (`Entry.only`). The gallery shows only the scenes in `YORU_REACTIONS` (`scenes.ts`, from `react` in `yoru/docs/protocol.md`) and plays them with `yoru_react`; Yoru's reactions without a scene here are buttons under "More on Yoru", on the board only. The mood card leaves out "Scenes", which `get_pet` doesn't give on a Yoru.
  - **Not yet.** The mirror and the Yoru tabs haven't been looked at in the running app.
- Step 10: flash and identify per board.
  - **Which port.** The pinned port, else the USB port of the board shown, else the only Espressif device. A blank board has a session too (it shows as "connecting…" in the switcher), so it's picked the same way, with the kind chosen for it; that choice is reset when another board is shown. A board shown on WiFi is refused: esptool needs USB.
  - **Only that board pauses** (`Manager::pause`, step 8). The window stays on the board being flashed instead of jumping to another (`Devices::current` waits for a board picked that's away), and the app no longer sends a "searching" event that made every board look gone.

## Left over

- Try it all with a Yoru and a DualEye at once: the voice floor (two boards hearing one wake word), the hub and MCP `board`, the switcher, flashing one while the other streams, and the WiFi path with the manager.
- The follow-the-pointer setting is shared by every board.
- `--board` in the CLI goes after the subcommand.

The rest is the plan.

## Board kinds

`core/src/profile.rs`: `enum BoardKind { DualEye, Yoru }` and a static `BoardProfile` for each. `BoardKind::from_hello` reads `hello.board`: `"dualeye"` (DualEye's `main/rpc.c`) or `"yoru-amoled-1.75c"`. A `hello` without a `board` means DualEye. Both boards use the same USB VID, so a board's kind is known only after its `hello`.

| Field | DualEye | Yoru | Hardcoded today in |
|---|---|---|---|
| mDNS service | `_dualeye._tcp` | `_yoru._tcp` | `core/src/wifi.rs:24` |
| Proof labels | `dualeye host` / `dualeye board` | `yoru host` / `yoru board` | `wifi.rs:278, 288` |
| Session key labels | `dualeye h2b` / `b2h` | `yoru h2b` / `b2h` | `wifi.rs:291-292` |
| Screens and faces | two, faces and rotation | one, no faces | `bridge.rs:574-588, 832-883`, `hub.rs:56`, `voice.rs:479, 536` |
| Music cover, media | yes | no (`snapshot.music` still feeds the pet) | `bridge.rs:531, 598-621` |
| Gaze | only while the eyes face shows | always (the board decides) | `bridge.rs:486-494, 666` |
| Timer screen | `ShowOn` | none | `timers.rs:71, 756` |
| Settings | faces and rotations from `get_state` | the `get_settings` table plus `set_setting` | |
| Agent prompt and MCP text | "two round displays", faces | "one round screen, eyes", `react` names | `agent.rs:40, 93, 166-191`, `mcp.rs:43-51` |
| Tools cache | `board-tools.json` | one file per kind | `link.rs:337` |
| Legacy version line | `{"dualeye":…}` | none | `firmware.rs:71-80` |
| Firmware | `firmware/dualeye/` | `firmware/yoru/` | `app/src-tauri/src/lib.rs:83, 1300, 1734` |

Behaviour that differs by kind goes behind a `BoardBehaviour` trait (`on_ready`, `tick(&mut Snapshot)`, `on_settings_changed`, `read_settings`), with `dualeye.rs` (today's face, rotation and cover code moved out of `bridge.rs`) and `yoru.rs`.

## Core

`Bridge` is replaced by a `Manager` that owns these parts:

- **Shared services, once:**
  - the sensor collector;
  - Claude usage, timers, music, presence;
  - pet, diary, quips;
  - the voice models and the hub.
- **One sampler thread.** It builds one snapshot per second and runs timers, alerts and quips. It publishes the snapshot to a watch slot. There is a single sampler so that a timer fires once, not once per board.
- **One `BoardSession` per board.** It holds today's `session()` without the sampling: link, reader, voice pipeline, speaker, gaze, and the board's `BoardBehaviour`.
- **Discovery, every few seconds:**
  - USB ports that no session owns get a pending session until its `hello`;
  - every paired WiFi board (`Paired` gains a `kind`) without a live session gets one;
  - one mDNS daemon browses both services.
- **USB preference.** When a USB session's `id` matches a WiFi session, the WiFi one closes, as `watch_usb` does today.

**Events.** Events become `ManagerEvent { board, kind, event }`. `board` is empty for events of the computer (snapshot, timer fired, Claude alert, quip). There are new events `BoardAdded` and `BoardRemoved`. `Settings` splits into one variant per kind.

`Bridge` stays as a thin wrapper (a Manager with one fixed port) for the CLI and tests.

### Policies

- **Wake word.** The first board to wake claims the voice floor. Another board that wakes meanwhile gets `voice/stop` and goes back to idle. The floor is released when the conversation ends, or after 30 s at most. Any wake still silences a ringing timer.
- **Who speaks timers, alerts and quips:** only one board, never an echo. It is chosen in this order:
  1. the board the timer was set from;
  2. the primary board (an app setting);
  3. the board with the latest conversation;
  4. the first one connected.

  Every DualEye still shows the countdown face.

## Hub, MCP, CLI

- **Hub.**
  - It keeps one attached link per board.
  - Its `hello` lists `boards`, and still returns `board` for old clients.
  - Every forwarded call takes an optional `board`: an id, or a kind as a shorthand.
  - Without one, the call goes to:
    1. the only board;
    2. the only board that has the tool;
    3. the primary board.

    Otherwise it fails with error `-32002`, "several boards: pass `board`".
- **MCP.**
  - Tools are the union over the boards, each with an optional `board` enum.
  - It adds a `list_boards` tool.
  - Its instructions are built from the connected kinds.
- **CLI.**
  - A global `--board <id|kind>`.
  - A new `companion boards` command.
  - `wifi pair` stores the kind.
  - Streaming runs the Manager; `--port` pins one session.

## App

- **Rust.**
  - `AppState` holds the Manager and a map of devices.
  - Per-board settings (port, faces, rotation, pointer) move to `boards: {id: …}`, plus `primary`.
  - Commands about a board take an optional `board`.
  - Yoru gets `yoru_settings`, `yoru_set_setting`, `yoru_react` and `yoru_show_menu`.
  - Flashing pauses only the session that owns the port.
- **Frontend.**
  - `monitor.svelte.ts` splits into a `Device` per board (link, voice, eyes, pet, firmware, settings) and the shared `Monitor` (snapshot, history, timers).
  - A device switcher sits in the title bar.
  - `Board.svelte` renders a DualEye and a new `YoruBoard.svelte` renders a Yoru.
  - Settings tabs are split into shared and per device. Yoru's device tab is built from its `get_settings` table: 0/1 settings as switches, ranges as sliders.
  - The bundled firmware is read with `import.meta.glob` over `firmware/*/`.

## Steps

Each step leaves the workspace compiling.

| # | Step | Size | Risk |
|---|------|------|------|
| 1 | New repository, names, `firmware/<board>/` | done | |
| 2 | `profile.rs`, wired into `wifi.rs` (labels, service, `Paired.kind`), the legacy probe, the tools cache, the agent and MCP texts; Yoru handshake test | done | |
| 3 | Embed both firmwares, `firmware_for(kind)`; refuse to flash an image whose project doesn't match | done | |
| 4 | Split `bridge.rs` into the sampler, `BoardSession` and `BoardBehaviour`, still with one session | done | |
| 5 | Manager: discovery, port ownership, USB over WiFi, `ManagerEvent` | done | freeing the port for esptool is left for step 8 |
| 6 | Voice floor and who speaks; serialise the STT and LLM sidecars if needed | done | |
| 7 | Multi-board hub, MCP `board`, CLI `--board` and `boards` | done | |
| 8 | Tauri commands with board ids, events, tray | done | |
| 9 | Frontend: `Device` store, switcher, `YoruBoard.svelte`, per-kind tabs | done | |
| 10 | Flash and identify per board (the user already picks the kind on a blank chip, since step 3) | done | |
