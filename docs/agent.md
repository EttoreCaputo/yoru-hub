# One agent, and a second brain for its memory

Decided on 2026-11-21; being built in phases (below). Yoru becomes one general agent with one system prompt. Everything Yoru does with a language model is a request to that agent, which works in a loop with tools until the job is done. Its memory is the transcripts of every conversation and meeting, searchable, with a second brain of Markdown notes on top, kept the way [COG](https://github.com/huytieu/COG-second-brain) (MIT) keeps one, and a small layer always in the prompt. Meetings feed it from their whole transcript, not only from their summary. Why this shape: [memory-research.md](memory-research.md).

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
- **Everything is a request.** A summary is "summarize meeting X" (in style Y), a title is "give this conversation a title", the nightly dreaming is "reread today and fix your notes". The same agent answers, with the same tools, under the same prompt. A request that runs on its own (no one in the chat) is a background run: it is not shown as a conversation, and what it did is logged in `yoru.log` and on the Activity panel.
- **A loop, ReAct-style.** The model calls tools, reads their results and continues until it answers or runs out of steps (a limit per request: 12 for a chat turn, 40 for a background run such as a meeting). Tools of one round run at once, as they do since 2026-11-08.
- **A base set, then more on demand**, as in Claude Code, OpenCode and Codex:
  - always loaded: the memory (`brain_search`, `brain_read`, `brain_write`, `brain_append`, `history_search`, `history_read`), `notify`, timers, `find_tools`, `load_tools`, `load_skill`, the web search, and the board's essentials (`react`, `get_state`);
  - loaded when the model looks for them (`find_tools`, `load_tools`, which already do this for the connectors): the app's own tools (below), the connectors' tools, and the board's other tools;
  - **skills**: Markdown files of instructions (a name and a one-line description in the frontmatter, then the body), with only the names and descriptions in the prompt (a `# Skills` section). The model loads one with `load_skill` when the request needs it. Built since 2026-11-21 (`assistant/skills.rs`): Yoru's own are built in (`core/src/assistant/skills/*.md`), and a file in the data folder's `skills/` adds one or replaces the built-in of the same name. The first is `summarize-meeting` (the styles and rules of `summary.rs`, and how to read the transcript and save the summary with the meeting tools); `process-meeting`, `people-profile` and `dream` come with the notes.
- **The app's own tools.** The agent can read and change every part of Yoru. They are found like a service's tools (service "Yoru": tool search on Claude, `find_tools` on the others), and what they change reaches the app's pages at once (`Assistant::on_app_change`). Built on 2026-11-21 (`assistant/app_tools.rs`):
  - meetings: `meeting_list`, `meeting_read` (15,000 characters a call, lines numbered, speakers by name), `meeting_summary`, `meeting_rename_speaker`, `meeting_save_summary`, `meeting_record` (everything played and the default microphone, as the window bar's Record) and `meeting_stop`, which the app runs (the listener answers);
  - to-dos: `todo_list`, `todo_set_done`. A to-do is a line of a meeting's summary, so there is no adding one on its own: something to do later is a task;
  - the conversations: `conversation_list`, `conversation_read` (in parts, like a transcript);
  - the settings: `settings_read` (the assistant's, routines included, and the meetings'), `settings_change` (only the fields that change, checked before the user is asked, then allowed or not on the card, like an action on a service);
  - already there before: the tasks (`tasks`), the inbox (`get_inbox`), the board's state and settings.
  The app's own settings (theme, voices, the notch) are kept by the app, not the core, and aren't reachable yet.
- **No other model behind it.** Whatever the user picked as the brain does everything (see [memory.md](memory.md), "One brain"). When the provider fails (Groq's limits, a wrong key, a model that refuses a parameter), the request fails: the user is told what the provider said and what to do (wait until when, check the key, pick another model), never switched silently to another model.

What stays outside the agent, because it isn't a language model's job:

- speech to text (audio in, text out: `stt.rs`, Gemini 3.5 Transcribe, Whisper, Parakeet) and text to speech;
- the voice prints (`voiceprint.rs`) and the embeddings (EmbeddingGemma);
- Gemini Live while it is the voice: it is a model of its own that hears and speaks. It gets the same system prompt and the same base tools; anything longer it hands to the agent (open question, below).

## The second brain

Yoru's memory becomes three layers, as Anthropic describes its own ("The components of agent memory", 2026) and as the research supports ([memory-research.md](memory-research.md)): the transcripts as the truth, notes on top, and a small layer always in the prompt. The notes are kept the way COG keeps its second brain.

### The transcripts: the foundation

- Every conversation (text and voice, from the app, the notch and the boards) and every meeting transcript stays as it is, where it is today: the chats in `chats/`, the meetings in their folders. Nothing is copied and nothing is deleted.
- They are indexed by turn, with the date, who spoke (the speaker's name from the voice prints and the diarization) and where it was said, for words (SQLite FTS5) and meaning (EmbeddingGemma). The index is a cache, built again when deleted.
- `history_search` finds turns by words, meaning, person and dates; `history_read` reads around a turn. The research is clear that keeping the original text and searching it well beats replacing it with extracted facts.
- Built on 2026-11-21 (`assistant/history.rs`): `history.db` in the data folder holds the turns (a chat line, or a meeting's consecutive lines by one speaker up to 1,500 characters, with the number of its first line), an FTS5 table for the words and EmbeddingGemma's first 256 dimensions for the meaning, fused with Reciprocal Rank Fusion. A conversation or meeting is indexed again when it changes (new lines, a speaker named, the reply language, which names the user), and dropped when deleted. The memory's two-minute tick indexes and embeds up to 512 turns; a search indexes what changed first, so it never misses the last lines, though their meaning waits for the tick. Both tools are always loaded; a hit is `meeting:<id>#<n>` or `chat:<id>#<n>` with the title, the date, who and the words, and `history_read` reads from four lines before it, 6,000 characters at a time.

### The notes: what Yoru knows

- `brain/` in the data folder: plain `.md` files with YAML frontmatter (`type`, `description`, `aliases`, `updated`) and `[[wikilinks]]`, so the folder also opens in Obsidian. Every change is a commit in a git repository inside it (git2, no shell): a bad change is undone, and the Memory page shows what changed and when.
- **One page per thing**, never a shared table (one page per item kept every fact in Anthropic's test, a shared table 72%):

```
brain/
├── me.md                  who the user is, strongest preferences, what not to say
├── index.md               one line per note: what it is about
├── people/<name>.md       one note per person
├── projects/<name>.md     one note per project
├── topics/<name>.md       places, organisations, interests, preferences on a subject
└── archive/               notes no longer true: moved here, never deleted
```

- Each note has, as in COG: **what is true now** on top, rewritten when the evidence changes, and a **timeline** below, dated entries only ever added to. Every claim cites the transcript it rests on (a link to the conversation or the meeting, at the turn), its date and how sure it is, and quotes the words that matter:

```markdown
- Head of design at Acme («I moved to Acme in September») [Source: [[meeting:2026-11-20 Weekly with Marco#t412]] | 2026-11-20 | confidence: high]
```

- What already exists elsewhere is not copied: a meeting's summary stays in the meeting, a conversation in the chats; notes link to them.
- People's notes grow in tiers (COG's): a stub at the first mention, more after three, a full profile after a meeting with them or eight mentions. Kinship (mother-in-law, brother-in-law) is still computed from a few relations in the people's frontmatter (`relations:`, read by `kin.rs`).

### The injected layer

- Every request carries `me.md` and `index.md`, a few KB at most (Claude Code loads 25 KB of its index; Hermes about 3.5 KB). The notes and the transcripts are read when needed. A wrong line here goes into every answer, so only dreaming promotes anything to it.

### Reading

- The agent is told what the memory is for and to look before answering about the user's life: people, projects, plans, what was said. It searches step by step (the index, the notes, then the transcripts for detail or what the notes lack) until it has enough. The research shows the bottleneck is using the evidence, not finding it, so the agent reads the source rather than trusting a line.
- Searches understand time ("last week", "in March") and people ("what did Marco say").

### Writing

- **During a conversation** the agent writes with its own judgment, when the user says something worth keeping or asks to remember (`brain_write`, `brain_append`, replacing `remember`, `forget`, `revise_memory`, `relate`). It is told what is worth keeping and what isn't ([memory.md](memory.md), Curation), not given a fixed rule: Anthropic measured fixed rules doing worse with strong models.
- **After a meeting** a background run with `process-meeting` reads the whole transcript, in parts, knowing who said what, and updates the notes of the people who were there and the projects discussed, citing the turns. The summary is written by the same run, with `summarize-meeting`, into the meeting.
- **Dreaming, every night** (`dream`): the day's transcripts are reread and the notes tested against them; wrong lines fixed, duplicates merged, new pages built, the timeline folded into what is true now, people's tiers raised, notes no longer true moved to the archive, `index.md` rewritten and kept small. It writes on a branch of the brain's repository and merges it when done, so a night can be reviewed or undone. What it did goes in `yoru.log` and in a notification.
- **Never kept**: what the user said in a chat with memory off, health, politics and religion unless the user asked, ID and account numbers (as the Claude app does).

### From today's memory

- The graph in `memory.db` is written out once as notes: each person and entity a note, its facts what is true now, with their episodes as sources, contradicted facts in the timeline. `memory.db` is kept, renamed, until the user deletes it.
- The voice prints stay where they are; a person's note links to its voice.
- The Memory page shows the notes: a list by kind, each readable and editable, its history, the sources behind each line, and a button to open the folder in Obsidian or the Finder. The graph drawing stays, built from the links.

### Measuring it

Public benchmarks are too noisy to choose by (the reader model and the judge move scores more than the designs do). Yoru gets its own: questions about the user's past conversations and meetings with their answers (who, what, when, what changed, what Yoru shouldn't know), run whenever the memory changes, `cargo test --ignored` style, on saved transcripts.

## Phases

1. **One prompt, skills and the app's tools.** `prompt.rs` replaces the three system prompts (done on 2026-11-21: Claude, the compatible brains and Gemini Live share it, and since the same day skills and `load_skill` work for all three; the model on this Mac, `agent.rs`, a small local model the board's voice falls back to, keeps its short one until it goes, since a silent fallback is another model behind the brain); `load_skill` and `skills/`; the tools for meetings, to-dos, tasks, routines, the inbox and the settings. Summaries and titles become requests to the agent; `summary_by` and the local summarizer go (done on 2026-11-21: summaries are `Assistant::summarize_meeting`, a background request of up to 40 steps, against 16 for a chat turn; a title is a background request answered in words, at most two steps, at low effort; `title.rs` keeps only when a conversation is due a name and the user's own title). Done when a meeting is summarized by the agent with the same quality, and a chat can answer "what did Marco say on Friday?" from the transcript.
2. **The transcripts searchable.** The index of every conversation and meeting turn (words, meaning, dates, speakers), `history_search` and `history_read`. (Built on 2026-11-21.) Done when the agent answers about a past conversation or meeting from the transcript.
3. **The notes.** `brain/`, its tools, git, `me.md` and `index.md` in the prompt, the migration of `memory.db`, writing during conversations, `process-meeting`. Done when the memory is the notes, nothing is lost from the graph, and a week leaves people and project notes with sources.
4. **Dreaming and the measure.** `dream` every night on a branch; Yoru's own questions and answers to measure the memory. The old pipelines (`sleep.rs`'s extraction, `summary.rs`'s prompts, `title.rs`) are removed.
5. **The Memory page** on the notes, with their history.

## Open questions

- Gemini Live: keep it as the voice with the base tools and hand anything longer to the agent, or speak the agent's answers with text to speech.
- The board's own memory, which today gets the graph in short: the same, from `me.md` and the people's snapshots.
- Small brains (Groq, Ollama) read 3,000 characters of memory and a few tools. The agent works with them as it can; when they fail, the user is told why.
