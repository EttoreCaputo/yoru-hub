# Yoru, the assistant

Decided on 2026-10-19. Yoru becomes a personal assistant that lives on the computer, and the round ESP32-S3 board (`../yoru`) is its body. It connects to the services you use (mail, calendar, documents, issues…) through MCP connectors, tells you when something worth knowing happens, and answers when you ask: "anything new?", "what's on tomorrow?", "find the mail from…".

This repository (`yoru-hub`, formerly `companion-hub`) becomes Yoru's only host program. DualEye leaves it (DualEye keeps its own `host/`), and so does everything that was there only for the pet. The Yoru app (`../yoru-app`, the fork of coucou) stays in use until phase A2 works, then it is retired.

Out of scope, on purpose: firmware 0.9's remote features on the host (drafts, session chats, new sessions) and the Mac-native parts of the Yoru app (pasting with Accessibility, bringing a terminal to the front, Parakeet, Apple's speech recognition). The board still works without them: its push-to-talk is an ordinary utterance.

## The parts

```
            ┌───────────── Yoru (this repository) ─────────────┐
 services   │  connectors ──► agent ◄── voice (stt, tts)        │      board
 (remote    │  (MCP client,    │  ▲                             │  (link: USB/WiFi)
  MCP, OAuth)│  OAuth, tokens) ▼  │                             │◄────────────►  Yoru
            │  watchers ──► triage ──► notifications ──────────►│
            │  memory, timers, coding agents (Claude Code hooks)│
            │  MCP server (for Claude Code and others)          │
            └──────────── app (Tauri + Svelte), CLI ────────────┘
```

## Connectors: our MCP client

Yoru is an MCP **client** of the services' servers, and stays an MCP **server** for Claude Code (`mcp.rs`).

`rmcp` 3.5 (already a dependency, server side only) has the client side too: the features `client`, `auth`, `transport-streamable-http-client-reqwest` and `transport-child-process`. Its `auth` module does the OAuth 2.1 work: discovery of the server's metadata, dynamic client registration, PKCE, token refresh and scope upgrades, behind a `CredentialStore` and a `StateStore` trait.

New in `core/src/connectors/`:
- `store.rs`: `CredentialStore` and `StateStore` on the system keychain (the `keyring` crate). Tokens never go into a plain file.
- `login.rs`: the OAuth redirect on a loopback address (`http://127.0.0.1:<port>/callback`, RFC 8252). It opens the browser, takes the code, and closes.
- `registry.rs`: the connectors in `connectors.json` in the data folder. Each one has its transport (a URL, or a command for a local server), a client id and scopes when the service needs our own, the tools turned on, and for each tool whether it only reads or acts.
- `manager.rs`: a tokio runtime on a thread of its own. It keeps the MCP sessions open, reconnects them, refreshes tokens, and lists the tools of every connector. The agent is synchronous (`ureq`, threads), so it reaches the manager through a blocking handle.

There are three kinds of service:
- **A remote MCP server with dynamic registration.** It works with nothing set up in advance. Several services publish one; whether each supports dynamic registration is checked service by service.
- **A service that needs our own OAuth client**: Google (Gmail, Calendar, Drive), GitHub, Slack, Box, Asana, HubSpot (none takes dynamic registration or client ID metadata documents, checked 2026-10-19). The app's own client for each is registered once and compiled in from `YORU_<SERVICE>_CLIENT_ID`/`_SECRET` (`docs/oauth-clients.md`), so Connect is one click there too. A build without it falls back to the user's own client, folded under "Advanced" on the card. Google: an app left in "testing" gets refresh tokens that expire after 7 days, and Gmail's scopes are restricted. Fine for personal use, but it shows up as a weekly sign-in until the app is verified.
- **A local MCP server** (`npx …`, `uvx …`), run as a child process, with its token in the keychain and passed as an environment variable.

## The agent

`agent.rs` already has the loop: tools, their results fed back, a short memory, a provider with a local fallback. What changes:
- **A model that can handle many tools.** A provider for Anthropic's Messages API next to the OpenAI-compatible one (`cloud/`). The local model stays for quick and offline requests.
- **Two brains.** The user picks one (`provider` in `assistant.json`; unset: Claude when it has a key, else Groq when it has one), see "The brain" below.
- **Tools per request.** The connectors that matter for the request, picked from their descriptions, plus Yoru's own: `notify`, `remember`, timers, the board's tools. Not every tool of every connector at once.
- **Acting asks first.** A tool marked as acting (send a mail, create an event, close an issue) shows on the board's approval card (`show_approval`, already in the firmware) and in the app, and runs only on Allow. Reading never asks.
- **Longer turns.** More rounds and tokens than a voice command; the spoken reply stays short, and the full answer is kept in the conversation in the app.
- **A character.** The cat personalities go. Yoru is useful and to the point, with Yoru's manners (`../yoru/docs/CONCEPT.md`).

## The brain

Decided on 2026-10-19: Yoru thinks with one of two brains, so people can start without paying.

| | Groq (`assistant/groq.rs`) | Claude (`cloud/anthropic.rs`) |
|---|---|---|
| Cost | free, with daily limits (free plan: 8K tokens a minute, 200K a day for GPT-OSS) | paid by use |
| Model | `groq_model`: `openai/gpt-oss-120b` by default; `openai/gpt-oss-20b`, `qwen/qwen3.8-27b`, `llama-3.3-70b-versatile` | `model`: `claude-opus-5-5` by default |
| Turn | `/chat/completions` with function calling: `tool_calls` run by the same tool runner, answered with `role: "tool"` messages; same rounds, same confirmation before acting | Messages API, thinking, effort |
| Connector tools | picked locally for each request (`assistant/select.rs`: BM25 over service, name and description against the request and the turns before, with a few Italian words mapped to English); 8 at most, plus those the conversation used, plus `find_tools` for the model to ask for more | all of them, deferred behind server-side tool search |
| Web search | none: Groq's compound models aren't used, and the model is told it can't search | the server tool |
| Prompt | a compact system prompt, the memory cut to 3,000 characters, the last 4 turns, tool results cut to 6,000 characters | the full prompt, cached |
| Triage | structured output (`response_format: json_schema`, strict) for GPT-OSS and Qwen; JSON mode with the schema in the prompt for Llama | `output_config.format` |

The conversation is kept in memory in the brain's own form (Anthropic's content blocks or OpenAI's messages); changing the brain starts a new one. The saved log (`conversation.json`) is the app's neutral `ChatLine`s, so nothing old breaks.

A 429 from Groq says what ran out (the minute's tokens or the day's) and when to retry (its `retry-after`, or the "try again in 7m12s" of its message). The model isn't asked again until then. The app shows it as an error with that time; said to the board, Yoru answers with a short spoken line ("I've used up today's free thinking. Try again in 13 minutes.") instead of a worse answer. Tests point Groq elsewhere with `GROQ_BASE_URL` (each provider's key variable with `_BASE_URL`), as `ANTHROPIC_BASE_URL` does for Claude.

The first run's brain step offers both, with their keys; its optional second part sets Yoru's voice: an ElevenLabs key, one of the account's voices (`elevenlabs:voice/<id>`, for both languages), and a test on the board. Settings has the brain, both keys and both models; `yoru settings provider groq|anthropic|auto` and `yoru settings groq_model …` do the same from the CLI.

## Memory

Files in the data folder that the agent reads at the start of a turn and writes with `remember`: who you are, what matters to you (people, projects, topics), what to tell you and what not, and what it has already told you. You read and edit them in the app.

## Proactive: watchers, triage, notifications

- **Watchers.** A periodic check without the model: it calls a connector's read tool with fixed arguments ("unread mail since…", "events in the next 2 hours"), compares the ids with the ones already seen, and wakes the model only for new ones. A watcher is data: connector, tool, arguments, where the id is, how often.
- **Triage.** The model decides, with the memory, whether something new is worth telling, how urgent it is, and says it in one line.
- **Delivery.** By urgency: a line in the app's inbox; a toast on the board; Yoru knocks on the glass; Yoru says it aloud. Quiet hours and presence (`presence.rs`) hold back the loud ones. `bridge/alerts.rs` already has the thread that waits for the right moment; `quips.rs` and `diary.rs` are the model for it.
- **Routines.** The morning recap (mail, calendar, what's pending) at a set time, and on request ("recap").
- **Firmware.** An inbox on the board: a `show_notification` tool (source, title, one line, urgency) and a list to scroll. It is the only real change to the board.

## Coding agents

`claude/agents.rs`, `claude/hooks.rs` and `bridge/agents.rs` stay: Claude Code's sessions and permission requests on the board, as one more source of notifications. Only Claude Code for now.

## The app: Yoru only, redone

Today the window is DualEye's: a home with the board's mirror and pills, and a settings drawer with Voice, Pet, Timers, Claude, Device and Diagnostics tabs. The new app has a sidebar:

| Page | What is on it |
|------|---------------|
| Today | Yoru's mirror, small; what it told you today; what comes next (events, timers, sessions at work); a field to type or talk to it |
| Conversation | The chat with Yoru, text and voice, with the tools it used folded away and the confirmations it is waiting for |
| Inbox | Every notification with its source, read or not, and its actions: open, reply, dismiss, "don't tell me about this again" (which goes into the memory) |
| Connectors | A card per service: connected or not, Connect and Disconnect, what it may read and what it may do, its watchers and how often |
| Routines | The morning recap, quiet hours, the watchers' schedule |
| Memory | What Yoru knows about you, editable |
| Yoru | The board: connection, WiFi, firmware, its settings, wake word, voice and volume |
| Settings | The model and its key, speech to text and voices, language, start at login, diagnostics |

The tray icon shows the last notification, a "talk" button and do-not-disturb. The first run has four steps: connect Yoru, pick the brain (with Yoru's voice as an optional part), connect a first service, say what matters to you (the first memory).

## What leaves this repository

- DualEye: its profile, faces, rotation, cover, `bridge/dualeye.rs`, `FacePicker`, `Board.svelte`, `Telemetry`, `Chart`, `Eye.svelte`, `eyes.ts`, `scenes.ts` (only Yoru's reactions stay), and the multi-board switcher (one Yoru at a time is enough).
- The pet of firmware 0.6: needs, feed, `get_pet`, `MoodPill`, `PetMood`, `PetGallery`, `petMood.ts`, the Pet tab, the pet parts of `quips.rs` and `diary.rs`.
- `sensors/`, `power-helper`, `snapshot.rs`'s metrics (`pointer.rs` stays: Yoru's eyes follow the mouse) (Yoru has no gauges), `music.rs` and `media.rs` (music comes back as a connector if wanted), `eval.rs`, and `intents.rs` unless it stays as the offline fallback.

## Phases

Status (2026-10-19): A0, A1, A2, A4 and A5 are built in the core and the CLI (`yoru`), with tests; the app (A3) is redone and checked in the browser preview, not yet in the Tauri window with a board. The board's inbox is in firmware 0.10.0. Not yet tried end to end with a real Anthropic key or a real OAuth sign-in.

- **A0. Yoru only.** Remove DualEye and the pet. Align with firmware 0.9.2 (no `get_pet`, push-to-talk accepted). The app, the CLI and the identifier are named Yoru. Done when the app runs with a Yoru on USB and nothing of DualEye is left.
- **A1. Connectors.** The MCP client, OAuth, the keychain store, `connectors.json`. Done when `yoru connect <url>` signs in to a remote MCP server and `yoru tools` lists its tools.
- **A2. The agent with connectors.** The Anthropic provider, tools per request, acting asks first on the board. Done when "what mail do I have?" said to Yoru gets a real answer. Then the Yoru app (`../yoru-app`) is retired.
- **A3. The new app.** The pages above, the Connectors page with Google and GitHub through our own OAuth clients, the first run.
- **A4. Watchers and notifications.** Watchers, triage, delivery, the inbox in the app, and `show_notification` with the inbox in the firmware.
- **A5. Memory and routines.** The memory files and their page, the morning recap, quiet hours.

## Open questions

- The default model: settled on 2026-10-19. Claude (`claude-opus-5-5`) when there is an Anthropic key, else Groq (`openai/gpt-oss-120b`, free) when there is a Groq key; the first run offers both.
- Which services first. Gmail and Calendar are the most useful and need our own Google OAuth app.
