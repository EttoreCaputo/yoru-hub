# Yoru, the assistant

Decided on 2026-10-19. Yoru becomes a personal assistant that lives on the computer, and the round ESP32-S3 board (`../yoru`) is its body. It connects to the services you use (mail, calendar, documents, issues…) through MCP connectors, tells you when something worth knowing happens, and answers when you ask: "anything new?", "what's on tomorrow?", "find the mail from…".

This repository (`yoru-hub`, formerly `companion-hub`) becomes Yoru's only host program. DualEye leaves it (DualEye keeps its own `host/`), and so does everything that was there only for the pet. The Yoru app (`../yoru-app`, the fork of coucou) was retired on 2026-10-19, once phase A2 worked.

Out of scope, on purpose: firmware 0.9's remote features on the host (drafts, session chats, new sessions), following coding agents (see below) and the Mac-native parts of the Yoru app (pasting with Accessibility, bringing a terminal to the front, Parakeet, Apple's speech recognition). The board still works without them: its push-to-talk is an ordinary utterance.

## The parts

```
            ┌───────────── Yoru (this repository) ─────────────┐
 services   │  connectors ──► agent ◄── voice (stt, tts)        │      board
 (remote    │  (MCP client,    │  ▲                             │  (link: USB/WiFi)
  MCP, OAuth)│  OAuth, tokens) ▼  │                             │◄────────────►  Yoru
            │  watchers ──► triage ──► notifications ──────────►│
            │  memory, timers                                   │
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
- **A service that needs our own OAuth client**: GitHub, Slack, Box, Asana, HubSpot (none takes dynamic registration or client ID metadata documents, checked 2026-10-19). The app's own client for each is registered once and compiled in from `YORU_<SERVICE>_CLIENT_ID`/`_SECRET` (`docs/oauth-clients.md`), so Connect is one click there too. A build without it falls back to the user's own client, folded under "Advanced" on the card.
- **Google (Gmail, Calendar)**: since 2026-10-22 Yoru Hub serves their tools itself, on the REST APIs, with a "Desktop app" client the user makes in their own Google Cloud project (`docs/oauth-clients.md`). A client of ours would need Google's verification for Gmail's restricted scopes; the user's own works with any account. Drive is left out for now.
- **A local MCP server** (`npx …`, `uvx …`), run as a child process, with its token in the keychain and passed as an environment variable.

Web search on every brain comes from Tavily (`tavily` in the catalog, added 2026-10-21): a remote server with dynamic registration, whose free plan gives 1,000 credits a month (a basic search costs 1, an advanced one 2). Its tools carry no read-only annotations, so the preset marks search, extract, crawl and map as reading; `tavily_research` (4 to 250 credits a call) and `tavily_feedback` start turned off. A watcher on `tavily_search` with `topic: "news"` and `time_range: "day"` follows a subject without the model; feeds stay the better source for fixed headlines, since they cost nothing. Exa (`exa`, added 2026-10-22) is a second search with no sign-in; Airtable, ClickUp and Miro joined the catalog the same day, all with dynamic registration.

## The agent

`agent.rs` already has the loop: tools, their results fed back, a short memory, a provider with a local fallback. What changes:
- **A model that can handle many tools.** A provider for Anthropic's Messages API next to the OpenAI-compatible one (`cloud/`). The local model stays for quick and offline requests.
- **Four brains.** The user picks one (`provider` in `assistant.json`; unset: the first with a key of Claude, OpenAI and Groq; Ollama only when picked), see "The brain" below.
- **Tools on demand.** Yoru's own (`notify`, `remember`, timers) and the board's essentials (`react`, `get_state`) are always there; a connector's tools and the board's other ones (volume, brightness, wake word, menu, settings) only once the model looks for them (tool search on Claude, `find_tools` and `load_tools` on the other brains). With many services connected, a request doesn't carry every tool of every connector.
- **Acting asks first.** A tool marked as acting (send a mail, create an event, close an issue) shows on the board's approval card (`show_approval`, already in the firmware) and in the app, and runs only on Allow. Reading never asks.
- **Longer turns.** More rounds and tokens than a voice command; the spoken reply stays short, and the full answer is kept in the conversation in the app.
- **A character.** The cat personalities go. Yoru is useful and to the point, with Yoru's manners (`../yoru/docs/CONCEPT.md`).

## The brain

Decided on 2026-10-19: Yoru thinks with one of two brains, so people can start without paying. On 2026-10-20 OpenAI (paid, cheap) and Ollama (a model on this computer, free and private) joined Groq in `assistant/compat.rs`, the turn for every brain that speaks OpenAI's chat completions. They differ in a budget (`compat::Budget`): Groq and Ollama get the tight one below (Groq counts the answer's reserved tokens against its 8K a minute, and a local model reads a short prompt best), OpenAI a roomy one (16 connector tools, the memory to 12,000 characters, the last 10 turns, results to 30,000 characters, 4,096 tokens an answer).

- **OpenAI** (`openai_model`, `gpt-6-luna` by default; `gpt-5.4-mini`, `gpt-5.6-terra`). Only models that call tools through Chat Completions: they do it with `reasoning_effort: "none"`, and want `max_completion_tokens` instead of `max_tokens`. GPT-6.1 Sol and Astra call tools only through the Responses API, which Yoru doesn't speak. Its key serves `whisper-1` too.
- **Ollama** (`ollama_model`, empty until the user picks one of the models Ollama lists at `/v1/models`). No key; `http://localhost:11434/v1`, or `OLLAMA_BASE_URL`. The model must support tools.

| | Groq, Ollama, OpenAI (`assistant/compat.rs`) | Claude (`cloud/anthropic.rs`) |
|---|---|---|
| Cost | free, with daily limits (free plan: 8K tokens a minute, 200K a day for GPT-OSS) | paid by use |
| Model | `groq_model`: `openai/gpt-oss-120b` by default; `openai/gpt-oss-20b`, `qwen/qwen3.8-27b`, `llama-3.3-70b-versatile` | `model`: `claude-opus-5-5` by default |
| Turn | `/chat/completions` with function calling: `tool_calls` run by the same tool runner, answered with `role: "tool"` messages; same rounds, same confirmation before acting | Messages API, thinking, effort |
| Connector tools | none loaded at first: the prompt names the connected services (a line each), the model searches their tools with `find_tools` (names and a line each, ranked by `assistant/select.rs`: BM25 over service, name and description, with a few Italian words mapped to English) and loads the ones it wants with `load_tools`; 6 stay loaded at most (16 on OpenAI), the oldest go first, and those the conversation used stay for follow-ups | all of them, deferred behind server-side tool search |
| Web search | only through the Tavily connector: Groq's compound models aren't used, OpenAI searches only through the Responses API, and without Tavily the model is told it can't search | the server tool |
| Prompt | a compact system prompt, the memory cut to 3,000 characters, the last 4 turns, tool results cut to 6,000 characters, tool descriptions to 300, 1,024 tokens an answer (2,048 for a triage) | the full prompt, cached |
| Triage | structured output (`response_format: json_schema`, strict) for OpenAI, GPT-OSS and Qwen; JSON mode with the schema in the prompt for Llama and Ollama | `output_config.format` |

The conversation is kept in memory in the brain's own form (Anthropic's content blocks or OpenAI's messages); changing the brain starts a new one. The saved log (`conversation.json`) is the app's neutral `ChatLine`s, so nothing old breaks.

A 429 from Groq says what ran out (the minute's tokens or the day's) and when to retry (its `retry-after`, or the "try again in 7m12s" of its message). The model isn't asked again until then. The app shows it as an error with that time; said to the board, Yoru answers with a short spoken line ("I've used up today's free thinking. Try again in 13 minutes.") instead of a worse answer. Tests point Groq elsewhere with `GROQ_BASE_URL` (each provider's key variable with `_BASE_URL`), as `ANTHROPIC_BASE_URL` does for Claude.

The first run's brain step offers Groq, OpenAI and Claude, with their keys (Ollama is picked later, in Settings); its optional second part sets Yoru's voice: an ElevenLabs key, one of the account's voices (`elevenlabs:voice/<id>`, for both languages), and a test on the board. Settings has the four brains, their keys and their models (and Ollama's thinking switch); `yoru settings provider …` and `yoru settings groq_model …` do the same from the CLI.

## Voice

The board streams what it hears after its wake word, with PWR held, or while it keeps listening after an answer (`core/src/voice.rs`).
- **Listening.** whisper.cpp's `whisper-server` on this computer (`stt.rs`, Whisper `base`, `small` or `large-v3-turbo`), or a provider's Whisper (Groq, OpenAI), falling back to the local one at the provider's limits. Italian or English, detected.
- **Speaking.** Piper on this computer (`tts.rs`, a voice per language), or a provider's voice: ElevenLabs (it acts out audio tags such as `[laughs]`) or Groq. A failed cloud voice hands over to a downloaded one for a minute. What ElevenLabs said is kept, one clip per sentence, on the Spoken page (`spoken.rs`), to play again or export as a zip.
- **Keep listening after an answer.** The board listens again without the wake word, for as long as the answer calls for (`voice/listen` with `wait_ms` and `pause_ms`, firmware 0.11.6). What is heard then may only close the conversation ("no, thanks") or acknowledge it ("ok"): fixed phrases, or EmbeddingGemma on a `llama-server --embedding` sidecar (`intent.rs`) telling a goodbye from a new request; anything else goes to the assistant.
- **When you finished.** A short pause on the board, or Pipecat's Smart Turn v3.2 (`turn.rs`, run with tract) hearing from the voice whether the sentence is finished, on each `utterance_pause`, then `voice/stop` (firmware 0.11.7).

## The board's apps

Firmware 0.11.0 has three apps in its menu that the host drives:
- **Timers** (`set_timers`, `timer_action`). Each board's session (`bridge/session.rs`) sends the whole list from `timers.rs` whenever it changes (`Timers::revision`), and on every handshake; the board counts down by itself. A tap there pauses, resumes or dismisses one, a long press cancels it (`Timers::act`). The snapshot's `timer` still goes too, for older firmware.
- **Habits** (`set_habits`, `habit_check`). `assistant/habits.rs` keeps them in `habits.json`: a name, a goal of times a day, today's count and the days its goal was reached, for the streak. The model has one `habits` tool (list, add, remove, done, undo), the app a section on the Routines page; the board gets today's list on every change and at midnight, and a tap there sets the count.
- **Errands** (`show_fetch`). Each call the model makes to a connector's tool (not the watchers' calls) is told to `bridge/errands.rs` (`Assistant::set_errands`), which shows it on every board on a thread of its own: `start` before the call, `done` or `failed` after it, with the service's name.

## Memory

Files in the data folder that the agent reads at the start of a turn and keeps up to date on its own as you talk (`remember`, `revise_memory`, `forget`): who you are, what matters to you (people, projects, topics), what to tell you and what not, and what it has already told you. You read and edit them in the app.

## Proactive: watchers, triage, notifications

- **Watchers.** A periodic check without the model: it calls a connector's read tool with fixed arguments ("unread mail since…", "events in the next 2 hours"), compares the ids with the ones already seen, and wakes the model only for new ones. A watcher is data: connector, tool, arguments, where the id is, how often.
- **Triage.** The model decides, with the memory, whether something new is worth telling, how urgent it is, and says it in one line.
- **Delivery** (`bridge/alerts.rs`). Every notification goes into the app's inbox and, with `show_notification`, into the board's at its urgency: `low` only in the inbox, `normal` with a banner and a chirp, `high` with Yoru knocking on the glass, `urgent` with the alarm, lighting the dark screen, and said aloud by the board that speaks once no conversation is on. In quiet hours anything below `urgent` arrives as `low`.
- **Routines.** The morning recap (mail, calendar, what's pending) at a set time, and on request ("recap").
- **Firmware.** An inbox on the board: a `show_notification` tool (source, title, one line, urgency) and a list to scroll, in firmware 0.10.0; `notification_action` comes back when it is read, dismissed or opened.

## Coding agents

Removed on 2026-10-21: Yoru no longer follows Claude Code. The hooks, the status line, the sessions' feed, the permission requests answered from the board or the app, the alerts about Claude ("Claude needs you", "Claude is done", the plan's limits) and the transcripts' usage are gone, and so are the board tools and notifications that served them (`set_agents`, `show_toast`, `show_question`, `hide_question`, drafts, chats, new sessions, `answer`, `shortcut`, `to` on `utterance_start`).

What stays:
- Yoru's MCP server (`mcp.rs`), for any MCP client, and the button that adds it to Claude Code's config (`claude/mcpconfig.rs`). A client can make Yoru react, speak, notify you and set timers.
- The Allow/Deny card (`show_approval`, `hide_approval`, the `approval` notification), now only for the assistant's own confirmations (`assistant/confirm.rs`, `bridge/confirm.rs`).
- `claude/cleanup.rs`: at start the app takes out of Claude Code's settings.json the hooks (`"<exe>" --claude-hook`) and the status line (`"<exe>" --claude-statusline`) older versions installed, matching only Yoru's own executable (`yoru-app`, `yoru`), and puts back the status line it had replaced. Until then the app binary ends at once, silently, when run with either flag.

## The app: Yoru only, redone

Until 2026-10-19 the window was DualEye's: a home with the board's mirror and pills, and a settings drawer with Voice, Pet, Timers, Claude, Device and Diagnostics tabs. The new app has a sidebar:

| Page | What is on it |
|------|---------------|
| Today | Yoru's mirror, small; what it told you today; what comes next (events, timers); a field to type or talk to it |
| Conversation | The chat with Yoru, text and voice, with the tools it used folded away and the confirmations it is waiting for |
| Inbox | Every notification with its source, read or not, and its actions: open, reply, dismiss, "don't tell me about this again" (which goes into the memory) |
| Connectors | A card per service: connected or not, Connect and Disconnect, what it may read and what it may do, its watchers and how often |
| Routines | The morning recap, quiet hours, the watchers' schedule |
| Memory | What Yoru knows about you, editable |
| Spoken | What ElevenLabs said, to play again, delete or export (added 2026-10-22) |
| Settings | A list of parts: the brain (model and key), the MCP server, voice (listening, speaking, keep listening, a goodbye, the end of a sentence), language, the board (connection, WiFi, firmware, its settings, wake word, volume; it was the Yoru page until 2026-10-21) and about |

The tray icon shows the last notification, a "talk" button and do-not-disturb. The first run has four steps: connect Yoru, pick the brain (with Yoru's voice as an optional part), connect a first service, say what matters to you (the first memory).

## What leaves this repository

Done (2026-10-19): none of the following is left in the code.


- DualEye: its profile, faces, rotation, cover, `bridge/dualeye.rs`, `FacePicker`, `Board.svelte`, `Telemetry`, `Chart`, `Eye.svelte`, `eyes.ts`, `scenes.ts` (only Yoru's reactions stay), and the multi-board switcher (one Yoru at a time is enough).
- The pet of firmware 0.6: needs, feed, `get_pet`, `MoodPill`, `PetMood`, `PetGallery`, `petMood.ts`, the Pet tab, the pet parts of `quips.rs` and `diary.rs`.
- `sensors/`, `power-helper`, `snapshot.rs`'s metrics (`pointer.rs` stays: Yoru's eyes follow the mouse) (Yoru has no gauges), `music.rs` and `media.rs` (music comes back as a connector if wanted), `eval.rs`, and `intents.rs` unless it stays as the offline fallback.

## Phases

Status (2026-10-29): A0 to A5 are built, in the core, the CLI and the app, and the app is Yoru's only host (the Yoru app was retired on 2026-10-19). Since then: four brains (2026-10-20), tools loaded on demand (2026-10-21), Google on its REST APIs (2026-10-22), the voice above (2026-10-21 to 2026-10-22), and firmware 0.11.0's apps (2026-10-29: the timers' list, habits, errands; see "The board's apps"). The board's side is in firmware 0.10.0 (inbox), 0.11.5 (only Yoru), 0.11.6 and 0.11.7 (listening).

Left:
- **Windows and Linux.** The release workflow builds only for macOS (Apple Silicon and Intel); the other two are off for now.
- **Habits outside the app.** The MCP server and the CLI have no habits tools yet; the model and the app do.
- **The release.** `v0.1.0` is on GitHub, refreshed by each push to `main`; `main` stops at 2026-10-20, so everything from 2026-10-21 on is only on `develop`.

- **A0. Yoru only.** Remove DualEye and the pet. Align with firmware 0.9.2 (no `get_pet`, push-to-talk accepted). The app, the CLI and the identifier are named Yoru. Done when the app runs with a Yoru on USB and nothing of DualEye is left.
- **A1. Connectors.** The MCP client, OAuth, the keychain store, `connectors.json`. Done when `yoru connect <url>` signs in to a remote MCP server and `yoru tools` lists its tools.
- **A2. The agent with connectors.** The Anthropic provider, tools per request, acting asks first on the board. Done when "what mail do I have?" said to Yoru gets a real answer. Then the Yoru app (`../yoru-app`) is retired.
- **A3. The new app.** The pages above, the Connectors page with GitHub through our own OAuth client and Google through the user's, the first run.
- **A4. Watchers and notifications.** Watchers, triage, delivery, the inbox in the app, and `show_notification` with the inbox in the firmware.
- **A5. Memory and routines.** The memory files and their page, the morning recap, quiet hours.

## Open questions

- The default model: settled on 2026-10-19. Claude (`claude-opus-5-5`) when there is an Anthropic key, else Groq (`openai/gpt-oss-120b`, free) when there is a Groq key; the first run offers both.
- Which services first. Gmail and Calendar are the most useful: done on Google's REST APIs with the user's own client (2026-10-22).
