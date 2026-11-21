<!--
  What Yoru knows (docs/agent.md, The notes): his notes, drawn as a graph,
  each note a node in its kind's colour, what is true of it as dots around
  it, and its [[links]] as the links. A click opens the note's card over the
  graph: what is true now, the note itself to fix in Markdown, what changed
  and when (the notes' git history), and the archive. Read again when you
  come back, since Yoru may have written something meanwhile.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { bubble } from "../bubble.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import MemoryGraph from "../ui/MemoryGraph.svelte";

  type Note = { path: string; kind: string; title: string; description: string; aliases: string[]; private: boolean; updated: string; lines: [string, string][]; links: string[] };
  type Change = { at: number; what: string };
  /** memory.db's people, for the voices Yoru knows. */
  type Voices = { entities: { id: number; name: string; aliases: string[]; is_user: boolean }[]; voices: number[] };

  const KINDS = ["person", "project", "topic"];
  const DRAWN = ["person", "place", "project", "organisation", "app", "topic", "thing"];
  /** A line's id in the graph: its note's and its place. */
  const LINE = 1000;

  const t = $derived(i18n.t.memory);
  let notes = $state<Note[]>([]);
  let people = $state<Voices>({ entities: [], voices: [] });
  let error = $state("");
  let selected = $state<number | null>(null);
  let lit = $state<number | null>(null);
  let obsidian = $state(false);

  async function load() {
    try {
      [notes, people] = await Promise.all([call<Note[]>("notes_views"), call<Voices>("memory_graph")]);
      if (selected !== null && !notes[selected - 1]) selected = null;
    } catch (e) {
      error = why(e);
    }
  }
  $effect(() => {
    void load();
    call<boolean>("has_obsidian").then((has) => (obsidian = has)).catch(() => {});
  });
  // What's said to Yoru (from his chat here) may change what he knows.
  $effect(() => {
    void store.chat.length;
    const again = setTimeout(load, 1500);
    return () => clearTimeout(again);
  });

  /** The voice in memory.db of note `n`'s person, by name or alias. */
  function voiceOf(n: Note): number | null {
    const names = [n.title, ...n.aliases].map((s) => s.trim().toLowerCase());
    const e = people.entities.find((e) => people.voices.includes(e.id) && (n.kind === "me" ? e.is_user : !e.is_user && [e.name, ...e.aliases].some((a) => names.includes(a.trim().toLowerCase()))));
    return e?.id ?? null;
  }

  // The notes as the drawing takes them: a note is an entity, a line a fact, a link a relation.
  const graph = $derived.by(() => {
    const id = new Map(notes.map((n, i) => [n.path, i + 1]));
    return {
      entities: notes.map((n, i) => ({ id: i + 1, name: n.kind === "me" ? "User" : n.title, kind: DRAWN.includes(n.kind) ? n.kind : "topic", placeholder: false, is_user: n.kind === "me", label: null })),
      facts: notes.flatMap((n, i) => n.lines.map(([section, text], j) => ({ id: (i + 1) * LINE + j, about: i + 1, section, text, to: null, importance: 0.5, private: n.private }))),
      relations: notes.flatMap((n, i) => n.links.filter((p) => id.has(p)).map((p, j) => ({ id: (i + 1) * LINE + j, a: i + 1, kind: "", b: id.get(p)!, to: null }))),
      voices: notes.flatMap((n, i) => (voiceOf(n) !== null ? [i + 1] : [])),
    };
  });

  const note = $derived(selected !== null ? (notes[selected - 1] ?? null) : null);
  const nameOf = (n: Note) => (n.kind === "me" ? t.me : n.title);
  const day = (ms: number) => i18n.date(ms, { day: "numeric", month: "short", year: "numeric" });
  const when = (ms: number) => i18n.date(ms, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  /** The note's lines by section, in the order they come. */
  const sections = $derived.by(() => {
    const out: { section: string; lines: { text: string; at: number }[] }[] = [];
    note?.lines.forEach(([section, text], at) => {
      const last = out.at(-1);
      if (last?.section === section) last.lines.push({ text, at });
      else out.push({ section, lines: [{ text, at }] });
    });
    return out;
  });

  let drawing = $state<MemoryGraph>();

  function pick(id: number | null, fact?: number) {
    selected = id;
    lit = fact !== undefined ? fact % LINE : null;
    adding = false;
    editing = false;
  }

  // The card's note: its text when edited, and its history.
  let editing = $state(false);
  let text = $state("");
  let changes = $state<Change[]>([]);
  $effect(() => {
    const path = note?.path;
    void note?.updated;
    changes = [];
    if (path) call<Change[]>("note_changes", { path }).then((c) => (changes = c)).catch(() => {});
  });
  async function edit() {
    if (!note) return;
    error = "";
    try {
      text = await call<string>("note_text", { path: note.path });
      editing = true;
    } catch (e) {
      error = why(e);
    }
  }
  async function save() {
    if (!note) return;
    error = "";
    try {
      await call("note_write", { path: note.path, content: text });
      editing = false;
    } catch (e) {
      error = why(e);
    }
    await load();
  }
  async function act(cmd: string, args: Record<string, unknown>) {
    error = "";
    try {
      await call(cmd, args);
    } catch (e) {
      error = why(e);
    }
    await load();
  }

  // A new note, from the head's button.
  let adding = $state(false);
  let newName = $state("");
  let newKind = $state("person");
  async function create() {
    if (!newName.trim()) return;
    error = "";
    try {
      const path = await call<string>("note_new", { kind: newKind, name: newName.trim() });
      newName = "";
      adding = false;
      await load();
      const at = notes.findIndex((n) => n.path === path);
      if (at >= 0) selected = at + 1;
    } catch (e) {
      error = why(e);
    }
  }

  /** Yoru's chat, opened on the note. */
  const tellYoru = () => bubble.show(note && note.kind !== "me" ? t.draftAbout(note.title) : "");
</script>

<section class="memory">
  <header class="head">
    <div class="what">
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    <div class="actions">
      <button class="icon-btn" title={t.fit} aria-label={t.fit} onclick={() => drawing?.fit()}><Icon name="scan" /></button>
      <button class="btn" onclick={() => act("notes_open", { obsidian })}><Icon name="file-text" size={15} />{obsidian ? t.obsidian : t.finder}</button>
      <button class="btn primary" onclick={() => ((adding = !adding), (selected = null))}><Icon name="plus" size={15} stroke={2.25} />{t.add}</button>
    </div>
  </header>

  {#if error}<p class="error-text err">{error}</p>{/if}

  <div class="stage">
    <MemoryGraph
      bind:this={drawing}
      {graph}
      {selected}
      onselect={pick}
      youName={t.you}
      relationName={() => ""}
      kindName={(k) => t.kinds[k] ?? k}
      sectionName={(k) => t.sections[k] ?? k}
      inset={adding || note ? (editing ? 492 : 372) : 0}
    />

    {#if adding}
      <div class="card add">
        <div class="card-top">
          <b>{t.newTitle}</b>
          <button class="icon-btn small" aria-label={t.close} title={t.close} onclick={() => (adding = false)}><Icon name="x" size={15} /></button>
        </div>
        <input type="text" placeholder={t.newName} bind:value={newName} {@attach (el: HTMLInputElement) => el.focus()} onkeydown={(e) => e.key === "Enter" && create()} />
        <div class="row">
          <select class="compact" bind:value={newKind}>
            {#each KINDS as k}<option value={k}>{t.kinds[k] ?? k}</option>{/each}
          </select>
          <button class="btn small primary" disabled={!newName.trim()} onclick={create}>{t.create}</button>
        </div>
      </div>
    {:else if note}
      <aside class="card inspector" class:wide={editing}>
        <div class="card-top">
          <span class="tag">{t.kinds[note.kind] ?? note.kind}</span>
          {#if note.private}<span class="tag" title={t.privateHint}><Icon name="lock" size={12} />{t.private}</span>{/if}
          {#if voiceOf(note) !== null}<span class="tag done">{t.voiceKnown}</span>{/if}
          <button class="icon-btn small close" aria-label={t.close} title={t.close} onclick={() => pick(null)}><Icon name="x" size={15} /></button>
        </div>

        <h2>{note.kind === "me" ? t.you("User") : note.title}</h2>
        {#if note.description && note.kind !== "me"}<p class="desc">{note.description}</p>{/if}
        <p class="faint meta">{note.path}{note.updated ? ` · ${t.updated(day(Date.parse(note.updated)))}` : ""}</p>

        {#if editing}
          <p class="faint meta hint">{t.editHint}</p>
          <textarea class="source" bind:value={text} spellcheck="false" {@attach (el: HTMLTextAreaElement) => el.focus()}></textarea>
          <div class="row">
            <button class="btn small primary" onclick={save}>{t.save}</button>
            <button class="btn small quiet" onclick={() => (editing = false)}>{t.cancel}</button>
          </div>
        {:else}
          {#each sections as s}
            {#if note.kind === "me" || sections.length > 1}<p class="section">{t.sections[s.section] ?? s.section}</p>{/if}
            <ul class="lines">
              {#each s.lines as line}
                <li class:lit={lit === line.at}><span class="text">{line.text}</span></li>
              {/each}
            </ul>
          {/each}
          {#if !note.lines.length}<p class="faint meta">{t.noneHint}</p>{/if}
          <div class="row">
            <button class="btn small" onclick={edit}><Icon name="pencil" size={14} />{t.edit}</button>
          </div>
        {/if}

        <h3>{t.changes}</h3>
        {#if changes.length}
          <ul class="changes">
            {#each changes as c}<li><span class="faint">{when(c.at)}</span> {c.what}</li>{/each}
          </ul>
        {:else}
          <p class="faint meta">{t.noChanges}</p>
        {/if}

        <div class="foot">
          <button class="btn link" onclick={tellYoru}><Icon name="message-circle" size={14} />{t.tellAbout(nameOf(note))}</button>
          <div class="row">
            {#if voiceOf(note) !== null}
              <ConfirmButton small label={t.forgetVoice} question={t.forgetVoiceConfirm(nameOf(note))} sure={t.forgetVoiceSure} onconfirm={() => act("memory_forget_voice", { id: voiceOf(note!) })} />
            {/if}
            {#if note.kind !== "me"}
              <ConfirmButton small label={t.archive} question={t.archiveConfirm(note.title)} sure={t.archiveSure} onconfirm={() => { const path = note!.path; selected = null; void act("note_archive", { path }); }} />
            {/if}
          </div>
        </div>
      </aside>
    {/if}
  </div>
</section>

<style>
  .memory {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  /* The page's head, as on every page, but over a graph that fills the rest. */
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 12px 16px;
    padding: 26px 28px 18px;
    border-bottom: 1px dashed var(--rim-2);
  }
  .what {
    flex: 1 1 280px;
    min-width: 0;
  }
  h1 {
    font: 700 var(--fs-title)/1.1 var(--display);
    letter-spacing: -0.015em;
  }
  .lede {
    margin-top: 6px;
    color: var(--haze);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .err {
    margin: 10px 28px 0;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  /* The card over the graph, on its right. */
  .card {
    position: absolute;
    top: 16px;
    right: 16px;
    width: 340px;
    max-width: calc(100% - 32px);
    /* Ends above Yoru, who waits at the bottom right. */
    max-height: calc(100% - 136px);
    overflow-y: auto;
    padding: 14px 16px 16px;
    border: 1px solid var(--rim-2);
    border-radius: var(--r-lg);
    background: var(--night);
    box-shadow: var(--lift);
    animation: in 180ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateX(8px);
    }
  }
  .add {
    display: grid;
    gap: 10px;
  }
  .card-top {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
  }
  .card-top .close,
  .add .card-top button {
    margin-left: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .inspector > .row {
    margin-top: 8px;
  }
  .meta {
    margin: 0 0 4px;
    font-size: var(--fs-meta);
  }
  h3 {
    margin: 18px 0 6px;
    padding-top: 14px;
    border-top: 1px dashed var(--rim-2);
    font: 600 var(--fs-meta)/1.2 var(--sans);
    color: var(--pearl);
  }
  .section {
    margin: 10px 0 2px;
    color: var(--dusk);
    font-size: var(--fs-small);
  }
  .lines {
    list-style: none;
    margin: 0 -8px;
    padding: 0;
  }
  .lines li {
    display: flex;
    align-items: center;
    gap: 4px;
    min-height: 32px;
    padding: 2px 4px 2px 8px;
    border-radius: var(--r-sm);
    transition: background 140ms;
  }
  .lines li:hover,
  .lines li.lit {
    background: var(--hover);
  }
  .lines .text {
    flex: 1;
    min-width: 0;
    padding: 4px 0;
    border: 0;
    background: none;
    color: var(--pearl);
    font: inherit;
    font-size: var(--fs-meta);
    line-height: 1.4;
    text-align: left;
    cursor: text;
  }
  span.text {
    cursor: default;
  }
  .foot {
    display: grid;
    gap: 10px;
    margin-top: 16px;
    padding-top: 14px;
    border-top: 1px dashed var(--rim-2);
    font-size: var(--fs-meta);
  }
  .foot .btn.link {
    justify-self: start;
    gap: 6px;
  }
  .inspector.wide {
    width: 460px;
  }
  h2 {
    font: 650 var(--fs-head)/1.3 var(--display);
    color: var(--pearl);
  }
  .desc {
    margin: 2px 0 4px;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .hint {
    margin-top: 10px;
  }
  .source {
    display: block;
    width: 100%;
    min-height: 320px;
    margin: 6px 0 8px;
    font: 12.5px/1.5 var(--mono, ui-monospace, monospace);
    resize: vertical;
  }
  .changes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 6px;
    font-size: var(--fs-meta);
    color: var(--pearl);
  }
  .changes .faint {
    margin-right: 4px;
  }
</style>
