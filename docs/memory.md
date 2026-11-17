# Yoru's memory: a graph of people and facts

Decided and built on 2026-11-08 (where it stands: How it works, below). Yoru's memory was one Markdown file (`memory.md`) put whole into every prompt; it is now a graph of the people and things in your life, with facts that know when they were true, searched for what each answer needs, consolidated after each conversation and tidied once a night. Yoru also knows who is speaking by their voice.

## What was wrong with the file

- The whole file goes into every prompt, cut from its head past the brain's budget (12,000 characters for Claude, OpenAI and Gemini, 3,000 for Groq and Ollama, 1,200 for the board): the Notes go first.
- Lines have no date and no source; nothing says a fact stopped being true.
- No search and no limit: a long memory is just cut. Duplicates are caught only when identical.
- `forget` removes every line that contains the text it is given.
- The Memory page saves the whole file and overwrites what the model remembered while it was open.
- Nothing is learnt unless the model calls `remember` during the conversation. Meetings never reach the memory. The board learns nothing.

## What we learnt from others

- **Zep / Graphiti** (arXiv 2501.13956, open source): episodes (the raw conversations), then entities and facts taken from them, then communities. Facts have a validity window: a fact contradicted is closed, not deleted, so the graph knows what was true before. Its gains are on questions about time.
- **Mem0g** (arXiv 2504.19413): entities, then relations as triples; search walks the entities and matches the question against the triples. The graph adds about 2% over its flat memory: the extraction matters more than the graph.
- **A-MEM** (NeurIPS 2025): each memory a note with keywords and links to similar ones; new memories update the old.
- **HippoRAG** (NeurIPS 2024): the question's concepts seed a Personalized PageRank over the graph, which finds memories linked only indirectly.
- **Letta (MemGPT)**: a few blocks always in the prompt; a "sleep-time" agent reorganizes the memory while the assistant is idle.
- **Bip** (palis.dev/bip, MIT): a desk robot whose brain runs on an M1, the closest to Yoru. "Memory is a database, not a prompt." One SQLite file with a graph of people (each person once, a few canonical relations, kinship words computed from who asks), facts and stories attached to each person, placeholders for people mentioned without a name, sqlite-vec and FTS5 indexes. Search fuses vector, words and the named people's facts with Reciprocal Rank Fusion, then reranks by similarity, recency (45-day half-life) and importance. It consolidates while idle. Private facts are kept back and names replaced by roles before a cloud call.
- Measured comparisons are few and often the vendors' own. One controlled study puts a graph at 89% against 76% for flat vector search with a strong extractor, and almost even (62% against 61%) with a small one: the graph is only as good as the model that builds it.

## Decisions

- **One brain.** Everything the memory asks a language model (extracting facts, merging, consolidating) goes to Yoru's brain, the one model the user picked. No second model for the memory, no fallback model.
- **One SQLite file**, `memory.db` in the data folder, with FTS5, written by our own small module on `rusqlite` (`core/src/assistant/memory/`). Vectors are kept as blobs and compared one by one, not with sqlite-vec: a personal memory is a few thousand facts, read in milliseconds. The Rust crates that do this (`sqlite-graphrag`, `grit-core`) are young; their ideas are taken, not their code.
- **Embeddings with EmbeddingGemma**, the model Yoru Hub already runs on this computer for `intent.rs`.
- **`memory.md` is imported**, as the first episode, so nothing is lost; afterwards it is no longer the memory.

## The graph

- **Episodes**: what the memory was learnt from, with date and source: a conversation (from the computer or a board), a meeting and its summary, a `remember`, an edit on the Memory page.
- **Entities**: people, places, projects, organisations, apps, topics. Each has a name, aliases, a type, a short summary and an embedding. Each person exists once.
- **People's relations**: only a few canonical ones are stored (parent, partner, sibling, works with, friend); words like mother-in-law or brother-in-law are computed when asked, from the speaker's side. "My wife" and "Sofia's mother" lead to the same node, so a fact changed there is changed everywhere.
- **Placeholders**: someone mentioned without a name ("my mother-in-law makes great bread") becomes a provisional node, merged into the real person once the graph can tell who it is.
- **Facts**: a subject, a relation, an object or a text, with `valid_from`, `valid_to`, the episode it came from, an importance and a confidence. A contradiction closes the old fact.
- **The core**: a few lines always in the prompt (who you are, your strongest preferences, what not to tell you), like Letta's blocks. The board gets the graph in short (How it works: The board).

## Search

For each turn:

1. Words (FTS5) and meaning (EmbeddingGemma) find the entities and facts the message touches; the people it names bring their facts.
2. The three lists are fused with Reciprocal Rank Fusion, then reranked by similarity, recency (45-day half-life, to be tuned) and importance.
3. The prompt gets the core and the best facts, within the brain's budget (about 20 to 40 facts), instead of a file cut from its head.

If that misses facts linked only through others, a Personalized PageRank step from the entities found (HippoRAG) comes next; measured first.

## Learning: consolidation

- At the end of a conversation (15 minutes quiet, as today), at the end of a meeting's summary, and in the nightly memory tidy, the brain rereads what happened, knowing who said what, and returns people, relations, facts and stories as JSON.
- The new is merged with the old: same person under another name, a fact repeated, a fact that closes another, a placeholder that is now someone.
- Meetings bring their decisions and things to do, linked to the people who were there.
- `remember`, `revise_memory` and `forget` become exact operations on a node or a fact. `forget` shows what it would remove before removing it.

## Privacy

What leaves the computer is the least it can be: before a cloud brain sees the memory, facts marked private stay home and, where it doesn't change the answer, names become roles ("your wife", "the client"). The memory itself never leaves the computer except the core sent to the boards.

## Knowing who speaks

A voice print for each person, built from what they say to Yoru and from the meetings' diarization (`core/src/meetings/transcript.rs`), which already tells voices apart. A request then comes with who said it, and facts are kept about the right person. Without voice prints there is one person, the user.

## How it works

What was built on 2026-11-08, G1, G2 and G4 below, in `core/src/assistant/memory/`.

- **The store** (`mod.rs`): episodes, entities (with aliases, gender, placeholders and the user's node), relations, facts in sections (about you, what matters to you, tell me about, don't tell me about, notes) with `valid_from` / `valid_to`, importance and a private flag. `memory.md` is imported once as the first episode and renamed `memory.md.imported`.
- **The prompt**: the core (about you, what to tell and not, and every important fact; 12,000 characters for Claude, OpenAI and Gemini, the brain's budget for Groq and Ollama, always, plus `<memory_found>`, the facts found for the message (30 at most, 4,000 characters). Facts and entities carry ids (`[f12]`, `[e4]`).
- **Privacy with a cloud brain** (anything but Ollama): private facts stay home, and the people who have a role for you that no one else has ("your wife"; "your friend" only when there is one) are written by that role in the core, in the facts found and in the turn's notes, unless the message itself calls them by name. Their kin-word aliases ("wife") don't count as names. The tools accept the role too ("your wife" finds her).
- **The board** (`Memory::board`, sent by `bridge/cloud.rs` in `cloud/set`): the graph in short, 1,200 characters at most and whole lines only: who you are and your core facts, then the people and things around you, most known first, each with its summary and two most important facts, at least a third of the room for them. No ids, no private facts, people with a role by it. The session checks every 30 s and sends it again when it changed, so the board follows the graph.
- **Search** (`search.rs`): FTS5 words, EmbeddingGemma's vectors (cosine 0.35 or more, when the model is installed) and the facts of the people named, fused with RRF (k = 60, 50 from each list), then weighed by recency (45-day half-life) and importance. Without EmbeddingGemma it is words and people only.
- **The tools**: `remember` (`text`, `about`, `section`, `important`, `private`), `revise_memory` (`id`, `text`: the old fact is closed, not deleted), `forget` (`ids` or `text`: by text it only lists what matches, so the model forgets by id), `recall` (`query`), `relate` (`person`, `relation`: parent, child, partner, sibling, friend, colleague, `of`, `gender`) and `this_is` (`name`, `is_user`: who is speaking, enrolling the voice just heard). Kinship words (mother-in-law, brother-in-law...) are computed from the asker's side (`kin.rs`).
- **The Memory page**: two views. **Graph** (`app/src/lib/ui/MemoryGraph.svelte`, SVG with its own spring simulation, no library): you in the middle, the people and things as glowing nodes linked by their relations, sized by how much Yoru knows of them, the facts as small dots around their owner (the Facts switch hides them); a node can be dragged and flung and springs back, the view pans, zooms and glides to a node when clicked, and Fit frames it all. With reduced motion it stays still. **List**: the people and things, each with its card (name, kind, gender, summary, aliases, relations, voice), its facts by section, editable one by one, with history, merge and remove.
- **Consolidation** (`sleep.rs`, `Assistant::start_sleep`, called the sleep in the code): every 2 minutes the episodes that have been quiet for 15 minutes (a conversation, a meeting's summary with its decisions and to-dos) are reread by the brain, which returns people, relations and facts as JSON, merged with the graph (a fact nearly identical to one known, cosine 0.93, is not added; one that contradicts closes the old). An episode that fails three times is given up. Once a night between 3 and 5, the nightly memory tidy: the brain tidies the graph (duplicates, placeholders now known, stale facts).
- **Who speaks** (`voice.rs`, `core/src/voiceprint.rs`): the `yoru-audio voiceprint` helper (FluidAudio's WeSpeaker, about 30 MB, downloaded after the first request said aloud, then kept running) prints every utterance alongside the transcription; it is someone's when the cosine with their print is 0.6 or more, and joins their print when it is 0.7 or more. Prints are also learnt from meetings: the microphone's voice is the user's, and a speaker the user names in a transcript becomes that person's voice. An unknown voice, once voices are known, is "someone Yoru doesn't know", and the model can ask and call `this_is`.
- **Not done**: Personalized PageRank (not needed yet); the board can't recall beyond what it is sent.

## Milestones

- **G1. The graph.** `memory.db`, the import of `memory.md`, the core, hybrid search in the prompt, the three tools made exact, the Memory page reading and editing entities and facts (no more whole-file saves). Done when every answer gets its facts by search and the old file is no longer read.
- **G2. Consolidation.** Consolidation after conversations, meetings and at night, placeholders merged, facts closed when contradicted, meetings' decisions in the memory. Done when a week of conversations leaves a tidy graph without any `remember`.
- **G4. Who speaks.** Voice prints from the meetings, the person recognised on the board and the computer, the board's own memory synced from the core.
- **G6. Verdicts** (2026-11-13). The user's verdict on each notification (`verdicts.rs`: kept, and Enough as a "Don't tell me about" fact).

## Sources

- Zep: arxiv.org/abs/2501.13956, github.com/getzep/graphiti
- Mem0: arxiv.org/abs/2504.19413
- A-MEM: arxiv.org/abs/2502.12110
- HippoRAG: arxiv.org/abs/2405.14831
- Letta: letta.com/blog/memgpt-and-letta
- Bip: palis.dev/bip, github.com/LeonardoPalis/bip-ai
- Graph against vector: falkordb.com/blog/agent-memory-for-long-running-agents; MemDelta, arxiv.org/abs/2606.29914
