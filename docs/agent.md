# One agent, and a second brain for its memory

Decided on 2026-11-21; not built yet. Yoru becomes one general agent with one system prompt. Everything Yoru does with a language model is a request to that agent, which works in a loop with tools until the job is done. Its memory becomes a second brain of Markdown notes, kept the way [COG](https://github.com/huytieu/COG-second-brain) (MIT) keeps one. Meetings feed it from their whole transcript, not only from their summary.

## Why

Today Yoru is several programs that each talk to a model in their own way:

| What | Where | Its own prompt |
|---|---|---|
| The assistant (chat, voice, the board) | `assistant/mod.rs`, `compat.rs` | three variants of who Yoru is: Claude, the OpenAI-compatible brains, the local model (`agent.rs`) |
| Gemini Live | `assistant/live.rs` | its own spoken prompt |
| Meeting summaries | `meetings/summary.rs`, `meetings/gemini.rs` | a summarizer with styles, and a model picked apart from the brain (`summary_by`) |
| Memory consolidation and the nightly tidy | `assistant/memory/sleep.rs` | two extraction prompts that return JSON |
| Conversation titles | `assistant/title.rs` | its own |
| Watchers | `assistant/watchers.rs` | its own |

Each knows only its slice. A summary doesn't know who the people in the meeting are. The memory learns from a meeting only its decisions and to-dos. The assistant can't open a meeting, a transcript or the to-do list. A change in how Yoru behaves has to be made in six places.

## The agent

- **One system prompt**, `assistant/prompt.rs`: who Yoru is (`identity`, unchanged), how he works (look in the second brain before answering about the user's life, cite where a fact comes from, ask before acting on someone's behalf), the core of the memory, and the list of skills. The body (board, notch, Mac) and the channel (voice or text) are one line each, not separate prompts.
- **Everything is a request.** A summary is "summarize meeting X" (in style Y), a title is "give this conversation a title", the end of a conversation is "process this conversation into the second brain", the nightly tidy is "tidy the second brain". The same agent answers, with the same tools, under the same prompt. A request that runs on its own (no one in the chat) is a background run: it is not shown as a conversation, and what it did is logged in `yoru.log` and on the Activity panel.
- **A loop, ReAct-style.** The model calls tools, reads their results and continues until it answers or runs out of steps (a limit per request: 12 for a chat turn, 40 for a background run such as a meeting). Tools of one round run at once, as they do since 2026-11-08.
- **A base set, then more on demand**, as in Claude Code, OpenCode and Codex:
  - always loaded: the second brain (`brain_search`, `brain_read`, `brain_write`, `brain_append`), `notify`, timers, `find_tools`, `load_tools`, `load_skill`, the web search, and the board's essentials (`react`, `get_state`);
  - loaded when the model looks for them (`find_tools`, `load_tools`, which already do this for the connectors): the app's own tools (below), the connectors' tools, and the board's other tools;
  - **skills**: Markdown files of instructions (`skills/<name>.md`: a name, a one-line description and the body), with only the names and descriptions in the prompt. The model loads one with `load_skill` when the request needs it. The first ones: `summarize-meeting` (the styles that are in `summary.rs` today), `process-meeting`, `process-conversation`, `people-profile`, `consolidate`, `brain-hygiene`, `daily-note`.
- **The app's own tools.** The agent can read and change every part of Yoru:
  - meetings: list, read (a transcript in parts, by speaker and time), rename a speaker, write the summary, record or stop;
  - to-dos: list, tick, add;
  - tasks, routines, the inbox, the conversations (list, read);
  - the settings: read all of them; a change goes through the confirmations (`confirm.rs`), like an action on a service;
  - the boards: their state and settings, as today.
- **No other model behind it.** Whatever the user picked as the brain does everything (see [memory.md](memory.md), "One brain"). When the provider fails (Groq's limits, a wrong key, a model that refuses a parameter), the request fails: the user is told what the provider said and what to do (wait until when, check the key, pick another model), never switched silently to another model.

What stays outside the agent, because it isn't a language model's job:

- speech to text (audio in, text out: `stt.rs`, Gemini 3.5 Transcribe, Whisper, Parakeet) and text to speech;
- the voice prints (`voiceprint.rs`) and the embeddings (EmbeddingGemma);
- Gemini Live while it is the voice: it is a model of its own that hears and speaks. It gets the same system prompt and the same base tools; anything longer it hands to the agent (open question, below).

## The second brain

Yoru's memory becomes a folder of Markdown notes, the way COG keeps one: no database as the truth, notes a person can read and change, history kept.

### Where and how

- `brain/` in the data folder: plain `.md` files with YAML frontmatter and `[[wikilinks]]`, so the folder also opens in Obsidian. Every change is a commit in a git repository inside it (git2, no shell), so nothing is ever lost and the Memory page can show what changed and when.
- The search index (SQLite FTS5 and EmbeddingGemma vectors, from today's memory) stays, but only as a cache built from the notes: deleted, it is built again.

```
brain/
├── me.md                 the core: who the user is, strongest preferences, what not to say (always in the prompt)
├── people/<name>.md      one note per person
├── projects/<name>.md    one note per project, with its people, decisions and open threads
├── topics/<name>.md      places, organisations, interests, preferences on a subject
├── meetings/<date> <title>.md   each meeting processed: summary, decisions, to-dos, links
├── daily/<date>.md       what happened that day: conversations, meetings, what was learnt
├── inbox/                what the agent wasn't sure where to put, until the tidy files it
└── archive/              notes no longer true or useful: moved here, never deleted
```

### Notes: what is true now, and how it got there

Every note about a person, project or topic has two parts, COG's (after gbrain's):

- **Compiled truth**, on top: the current best understanding, rewritten when the evidence changes.
- **Timeline**, below: dated entries, newest first, only ever added to.

Every claim cites its source, its date and how sure it is:

```markdown
- Works at Acme as head of design [Source: [[meetings/2026-11-20 Weekly with Marco]] | 2026-11-20 | confidence: high]
```

`high` is something said outright, `medium` something seen across several talks, `low` a single hint or something heard from someone else. The frontmatter carries the note's `type`, `aliases`, `updated`, `last_verified` and `confidence`.

People's notes grow in tiers, as COG's do: a stub at the first mention (name, who they are, one line), more after three, a full profile after a meeting with them or eight mentions. Kinship (mother-in-law, brother-in-law) is still computed from a few stored relations (`kin.rs`), now read from the people's frontmatter (`relations:`).

### How it is fed

- **Brain first.** Before answering about the user's life (people, projects, plans, what was said), the agent searches the second brain. The prompt carries `me.md` and the notes the search finds for the message, as today's facts.
- **During a conversation** the agent writes when the user tells it something worth keeping, with `brain_write` and `brain_append` (they replace `remember`, `forget`, `revise_memory`, `relate`).
- **After a conversation** (15 minutes quiet, as today) a background run with `process-conversation`: the day's note, then the people, projects and topics touched.
- **After a meeting** a background run with `process-meeting`, on the **whole transcript**, read in parts, knowing who said what: the meeting's note (the summary, with the style asked), its decisions and to-dos, the people who were there (their notes created or updated, with the meeting as source), the projects discussed, and the day's note. This replaces today's summary pipeline and its own model.
- **Every night** `consolidate`: notes in `inbox/` filed, duplicates merged, the compiled truth rewritten from the timeline, people's tiers raised. **Every month** `brain-hygiene`: claims checked again where they can be, `last_verified` and `confidence` stamped, notes no longer true proposed for the archive in a notification (moved only once the user says yes).
- What is not worth keeping stays as [memory.md](memory.md) spells it out (small talk, requests already carried out, what Yoru said, transcription noise, feelings guessed from one remark). It goes into the skills.

### From today's memory

- The graph in `memory.db` is written out once as notes: each person and entity a note, its facts the compiled truth with their episodes as sources, contradicted facts in the timeline. `memory.db` is kept, renamed, until the user deletes it.
- The voice prints stay where they are (they are vectors, not notes); a person's note links to its voice.
- The Memory page shows the notes: a list by kind, each note readable and editable, its history, and a button to open the folder in Obsidian or the Finder. The graph drawing stays, built from the links.

## Phases

1. **One prompt, skills and the app's tools.** `prompt.rs` replaces the three system prompts; `load_skill` and `skills/`; the tools for meetings, to-dos, tasks, routines, the inbox and the settings. Summaries and titles become requests to the agent; `summary_by` and the local summarizer go. Done when a meeting is summarized by the agent with the same quality, and a chat can answer "what did Marco say on Friday?" from the transcript.
2. **The second brain.** `brain/`, its tools, git, the index built from the notes, the migration of `memory.db`, brain-first search in the prompt. Done when the memory is the notes and nothing is lost from the graph.
3. **Feeding it.** `process-conversation` and `process-meeting` (from the whole transcript), the daily notes, the people's tiers. Done when a week of conversations and meetings leaves people and project notes with sources.
4. **Keeping it.** `consolidate` every night, `brain-hygiene` every month, the archive proposals. The old pipelines (`sleep.rs`'s extraction, `summary.rs`'s prompts, `title.rs`) are removed.
5. **The Memory page** on the notes, with their history.

## Open questions

- Gemini Live: keep it as the voice with the base tools and hand anything longer to the agent, or speak the agent's answers with text to speech.
- The board's own memory, which today gets the graph in short: the same, from `me.md` and the people's snapshots.
- Small brains (Groq, Ollama) read 3,000 characters of memory and a few tools. The agent works with them as it can; when they fail, the user is told why.
