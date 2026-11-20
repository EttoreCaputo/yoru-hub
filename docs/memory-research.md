# Agent memory: what the research and the products show

Gathered on 2026-11-21, for the plan in [agent.md](agent.md). Two questions: which way of building an agent's long-term memory is best demonstrated by recent papers, and how the leading products (Claude above all) build theirs. Numbers are as the sources report them. "Vendor" marks a result published by whoever sells the system.

## The benchmarks, and how far to trust them

- **LoCoMo** (ACL 2024): about 26k tokens per conversation. Putting the whole conversation in context already scores about 73%, so the benchmark separates systems poorly. About 6% of its gold answers are wrong (an audit by Penfield Labs; the Mnemosyne paper, arXiv 2510.08601).
- **LongMemEval** (ICLR 2025, arXiv 2410.10813): 500 questions over about 115k tokens, testing extraction, multi-session reasoning, time, updates and abstention. Long-context models lose about 30% against an oracle. What helped: indexing per session or turn, facts added to the retrieval keys, and query expansion that knows about time.
- **BEAM** (ICLR 2026, arXiv 2510.27246): conversations of 128K to 10M tokens. Even 1M-token windows degrade as the conversation grows.
- **MemoryAgentBench** (ICLR 2026, arXiv 2507.05257): the strongest independent comparison, with one protocol for everyone. Commercial memory products did worst (Mem0 about 21–24 overall). On retrieval, a long-context model (71.8) beat HippoRAG-v2 (65.1) and BM25 (60.5). Multi-hop conflict resolution is at most 7% for every method.
- **Multi-party conversations**, the closest to meetings (all synthetic): with speakers to attribute, multi-hop answers reach 26% even with the right evidence (EverMemBench, arXiv 2602.01313). The best system averages 46%, 27% on updates, and plain BM25 matches or beats most memory systems (GroupMemBench, arXiv 2605.14498).
- **Diagnostics**:
  - When a system fails, the evidence was retrievable about ten times more often than missing (MemTrace, arXiv 2606.17328).
  - On LongMemEval-S the reader model alone moves the score from 93 to 479 of 500 on the same evidence, and a different judge moves it as much as papers' gains ("Auditing Long-Term Memory Evaluation", arXiv 2609.38021).

So differences of a few points between systems are noise. Most scores above 85 on LoCoMo or LongMemEval are reported by the system's own vendor (Mem0 and Zep each claim over 92, under setups that can't be compared, after a public dispute over each other's numbers).

## What the papers show

### Strong evidence (independent, or several papers agree)

1. **Retrieval beats putting everything in context, once the history is long** (above about 100k tokens): BEAM, LongMemEval, MemoryAgentBench. On short histories the whole context is as good or better.
2. **Simple retrieval over the raw history, cut fine and dated, is a very strong baseline.**
   - Plain RAG, top 20, scores 80% on LongMemEval-S with GPT-4o, against 71% for Zep (Mastra's table, vendor).
   - BM25 matches most memory products (GroupMemBench, MemoryAgentBench).
   - An agent with only grep and file search over the raw history scores 74% on LoCoMo, against 68.5% for Mem0's graph (Letta, vendor).
3. **Keep the original text: extraction loses information.** With everything else held fixed, verbatim chunks beat facts extracted by a model:
   - 43.9% against 28.0% on LoCoMo;
   - 67.4% against 45.4% on LongMemEval-S.

   Extracted facts added beside the chunks cost nothing; replacing them is what hurts ("Fidelity Before Structure", Tao An, EACL 2027, arXiv 2601.00821).
4. **Using the evidence is the bottleneck, not finding it.** A stronger model and an agent that searches again until it has enough give the largest gains (MemTrace; the audit; the same system moves from 84 to 95 by changing the model).
5. **Dates on everything.** A date on every item, and searches that understand time, help with time and with facts that changed (Zep, LongMemEval). Updates across several hops, and who said what among several speakers, are still unsolved.

### Weaker evidence (self-reported)

6. **The best systems of 2026 all report 91 to 95% with the same shape.**
   - The shape:
     - the raw history kept, dated, as the truth;
     - facts, timelines and profiles of people built from it in the background, each linked back to its source;
     - an agent that searches step by step when asked.
   - The systems: Mnemon (arXiv 2609.36059), Agent Zero Memory (arXiv 2608.29606), Hindsight (arXiv 2512.12818), Mastra Observational Memory, DeferMem (arXiv 2605.22411).
7. **Graphs show small gains over weak baselines.** Mem0's graph adds 1.5 points over Mem0. No controlled study shows a graph beating good hybrid search over the raw history. The same holds for memory managers trained with reinforcement learning (Memory-R1) and for reconsolidation.
8. **Forgetting and decay have no accuracy evidence.** Deleting old memories has not been shown to help; apparent forgetting over time is mostly retrieval failure (NeurIPS 2026). Background consolidation is shown to save tokens (LightMem: up to 38 times fewer), not to answer better.

## How the products build memory

- **Anthropic's own account** ("The components of agent memory", Lance Martin, 6 Nov 2026):
  - **Four layers:**
    1. transcripts as a searchable log;
    2. notes Claude writes;
    3. representations such as a wiki of linked pages;
    4. a small injected layer in every session.
  - **Dreaming:** an offline pass that rereads the transcripts to test the notes against them, fix the wrong ones, build the wiki, and decide what goes into the injected layer.
  - **Measured:**
    - Memory raises Opus 5.5 from 0.655 to 0.838 on CL-Bench.
    - Wiki plus search answers 61% against 53% for search alone, with about 20k fewer tokens per question.
    - One page per contract, quoting its terms word for word, kept 57 of 57 facts; a shared table kept 41.
    - Telling the model to "record only failures" made memory worse (−0.234 on Opus 5): strong models judge what matters better than fixed rules do.
    - In Pokémon, 413 files across 225 folders earned one badge; ten files earned four.
  - **Its advice:**
    - Say what memory is for, not how to use it.
    - Keep the injected layer small, since a wrong note there goes everywhere.
    - Save what can't be read again elsewhere.
    - Give each thing its own page.
    - Aim dreaming at what changes.
    - Build on transcript search, with the wiki on top.
- **Claude Code**: `CLAUDE.md` files written by the user, apart from what Claude learns. Auto memory is a `MEMORY.md` index (one line per memory, the first 200 lines or 25 KB loaded) and one file per memory, with a type in its frontmatter (user, feedback, project, reference), read when needed and written during the session. What the code or `CLAUDE.md` already says isn't saved.
- **Claude's API**:
  - **Memory tool** (`memory_20250818`): the model reads and writes files under `/memories` and is told to look there before anything else. With context editing it gained 39% on agentic search.
  - **Managed Agents' memory stores**: every write is a version, and stores can be mounted read-only for untrusted input.
  - **Dreams** (research preview): read a store and up to 100 transcripts and write a new store. The old one is untouched, so the result can be reviewed or thrown away.
- **The Claude app**:
  - Memories are topics saved during the chat; past chats are searched separately, and each project has its own memory.
  - Incognito chats are neither saved nor searched.
  - Health, politics and religion are left out unless the user opts in, and IDs and account numbers are never saved.
  - Deleting a chat doesn't delete what was learnt from it.
- **ChatGPT**:
  - Saved memories, plus a summary of the chat history kept up to date.
  - Since 2025–2026 it ranks memories by recency and frequency, lets them be pinned, updates conflicting ones on its own, and shows the sources behind an answer.
- **Codex**: memories built only in the background, once a chat is idle: one extraction per chat, then a global consolidation. The rules stay in `AGENTS.md`.
- **Letta**:
  - Memory is a git repository of Markdown files. The root files are always in the prompt, and folders with their own index are read when needed.
  - Dreaming runs in the background on a copy, and a second agent reviews its edits.
- **Hermes, OpenClaw**:
  - A small `MEMORY.md` (and `USER.md`) always loaded.
  - Past sessions searched with SQLite FTS5.
  - Daily notes only indexed, not loaded.
  - A dreaming pass that distills them into `MEMORY.md`, and a memory flush before compaction.
- **Cursor** and Devin Local dropped automatic memory for rules and skills (forum posts, not confirmed officially).

### Where they converge

1. Plain files the user can read and correct, not opaque vectors.
2. Two tiers: a small layer always loaded (an index or a profile, a few KB) and the rest read when needed.
3. One thing per file, with a type and a description, and the index points to them.
4. The transcripts kept and searchable as the truth; notes on top, never instead.
5. Written during the conversation by the model's own judgment, then consolidated in the background ("dreaming"), which checks notes against the transcripts, merges, fixes and keeps the top layer small.
6. A fact that changed: the latest wins at consolidation, with dates on everything.
7. Controls: memory per scope, chats with no memory, sensitive subjects left out, versions of every write, the sources of an answer shown.
8. What the user writes as rules is kept apart from what the agent learns.

## What this means for Yoru

- **The transcripts are the memory's foundation.** Every conversation and every meeting transcript, cut by turn, dated, with who spoke, and searchable with words and meaning (FTS5 and EmbeddingGemma, which Yoru already has). Nothing is deleted.
- **Notes on top, never instead.** One page per person, project or topic, with what is true now on top and dated entries below, each citing the transcript it comes from and quoting the words that matter. Summaries and conversations aren't copied into notes: they already exist and are linked.
- **A small layer always in the prompt**: `me.md` and an index of the notes, one line each, a few KB at most.
- **The agent decides what to keep**, told what memory is for and what isn't worth keeping, not given a fixed rule.
- **The agent searches step by step** (notes, then transcripts, then again) before answering about the user's life. The model the user picks matters more than the structure.
- **Dreaming at night**: reread the day's transcripts, test the notes against them, fix, merge, build pages, keep the top layer small. Every change is a git commit, so a bad night can be undone.
- **No graph database, no forgetting by decay**: neither is shown to help. What is no longer true goes to the archive, not away.
- **Who said what in meetings** is where every system is weakest: the transcript keeps the speakers' names and voices, and a note cites the speaker.
- **Measure on Yoru's own data**: public benchmarks are too noisy to choose by. A set of questions about the user's own past conversations and meetings, with the answers, checked whenever the memory changes.
