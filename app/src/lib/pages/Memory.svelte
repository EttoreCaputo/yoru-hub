<!--
  What Yoru knows, as a graph (docs/memory.md): the people and things in
  your life, what is true of each, and what used to be. Yoru writes it as you
  talk and while it rests; here you correct it, in the graph itself: a click
  on a node (or one of its facts) opens its card over the graph, where its
  name, links and facts are changed in place. Or you tell Yoru, from his
  chat at the bottom right. Read again when you come back, since Yoru may
  have learnt something meanwhile.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { bubble } from "../bubble.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import MemoryGraph from "../ui/MemoryGraph.svelte";

  type Fact = { id: number; about: number | null; section: string; text: string; from: number; to: number | null; importance: number; private: boolean; episode: number | null };
  type Entity = { id: number; name: string; kind: string; aliases: string[]; summary: string; gender: string | null; placeholder: boolean; is_user: boolean; label: string | null };
  type Relation = { id: number; a: number; kind: string; b: number; from: number; to: number | null };
  type Graph = { entities: Entity[]; facts: Fact[]; relations: Relation[]; voices: number[] };

  const SECTIONS = ["about", "matters", "tell", "quiet", "notes"] as const;
  const KINDS = ["person", "place", "project", "organisation", "app", "topic", "thing"];
  const RELATIONS = ["parent", "child", "partner", "sibling", "friend", "colleague"];

  const t = $derived(i18n.t.memory);
  let graph = $state<Graph>({ entities: [], facts: [], relations: [], voices: [] });
  let error = $state("");
  let selected = $state<number | null>(null);
  /** The fact clicked in the graph, lit in the card. */
  let lit = $state<number | null>(null);

  async function load() {
    try {
      graph = await call<Graph>("memory_graph");
      if (selected !== null && !graph.entities.some((e) => e.id === selected)) selected = null;
    } catch (e) {
      error = why(e);
    }
  }
  $effect(() => {
    void load();
  });
  // What's said to Yoru (from his chat here) may change what he knows.
  $effect(() => {
    void store.chat.length;
    const again = setTimeout(load, 1500);
    return () => clearTimeout(again);
  });

  /** Run `cmd`, then read the graph again. */
  async function act(cmd: string, args: Record<string, unknown>) {
    error = "";
    try {
      await call(cmd, args);
    } catch (e) {
      error = why(e);
    }
    await load();
  }

  const user = () => graph.entities.find((e) => e.is_user);
  const entity = $derived(graph.entities.find((e) => e.id === selected) ?? null);
  const aboutOf = (f: Fact) => f.about ?? user()?.id ?? null;
  const factsOf = (id: number) => graph.facts.filter((f) => aboutOf(f) === id && (history || f.to === null));
  const nameOf = (id: number) => {
    const e = graph.entities.find((x) => x.id === id);
    return e ? (e.is_user ? t.you(e.name) : e.name) : `#${id}`;
  };
  const relationsOf = (id: number) => graph.relations.filter((r) => r.to === null && (r.a === id || r.b === id));
  const day = (ms: number) => i18n.date(ms, { day: "numeric", month: "short", year: "numeric" });

  // What the graph shows, remembered on this computer.
  const read = (key: string, fallback: string) => {
    try {
      return localStorage.getItem(key) ?? fallback;
    } catch {
      return fallback;
    }
  };
  let showFacts = $state(read("memory.facts", "1") === "1");
  let history = $state(false);
  $effect(() => {
    try {
      localStorage.setItem("memory.facts", showFacts ? "1" : "0");
    } catch {
      // Not kept: fine.
    }
  });
  let drawing = $state<MemoryGraph>();

  function pick(id: number | null, fact?: number) {
    selected = id;
    lit = fact ?? null;
    adding = false;
  }

  // Someone or something new, from the head's Add.
  let adding = $state(false);
  let newName = $state("");
  let newKind = $state("person");
  async function addEntity() {
    if (!newName.trim()) return;
    error = "";
    try {
      selected = await call<number>("memory_add_entity", { name: newName.trim(), kind: newKind });
      newName = "";
      adding = false;
    } catch (e) {
      error = why(e);
    }
    await load();
  }

  // The card: the one selected, its fields as typed.
  let card = $state<Entity | null>(null);
  let aliases = $state("");
  $effect(() => {
    card = entity ? { ...entity, aliases: [...entity.aliases] } : null;
    aliases = entity?.aliases.join(", ") ?? "";
  });
  const dirty = $derived(
    !!card && !!entity && (card.name !== entity.name || card.summary !== entity.summary || card.kind !== entity.kind || card.gender !== entity.gender || aliases !== entity.aliases.join(", ")),
  );
  async function saveEntity() {
    if (!card) return;
    const list = aliases.split(",").map((a) => a.trim()).filter(Boolean);
    await act("memory_put_entity", { entity: { ...card, aliases: list, placeholder: card.placeholder && card.name === entity?.name } });
  }
  function fieldKey(e: KeyboardEvent) {
    if (e.key === "Enter") void saveEntity();
  }

  // A fact being corrected, and a new one.
  let editing = $state<number | null>(null);
  let editText = $state("");
  async function saveFact(f: Fact) {
    if (editText.trim() && editText.trim() !== f.text) await act("memory_put_fact", { fact: { ...f, text: editText.trim() } });
    editing = null;
  }
  const flag = (f: Fact, change: Partial<Fact>) => act("memory_put_fact", { fact: { ...f, ...change } });
  let draft = $state({ text: "", section: "about" as string });
  async function addFact() {
    if (!draft.text.trim() || selected === null) return;
    await act("memory_put_fact", { fact: { id: 0, about: selected, section: draft.section, text: draft.text.trim(), from: 0, to: null, importance: 0.5, private: false, episode: null } });
    draft = { ...draft, text: "" };
  }

  let relKind = $state("partner");
  let relTo = $state<number | "">("");
  let mergeInto = $state<number | "">("");

  /** Yoru's chat, opened on what's selected. */
  const tellYoru = () => bubble.show(entity && !entity.is_user ? t.draftAbout(entity.name) : "");
</script>

<section class="memory">
  <header class="head">
    <div class="what">
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    <div class="actions">
      <button class="chip small" class:on={showFacts} aria-pressed={showFacts} onclick={() => (showFacts = !showFacts)}>{t.showFacts}</button>
      <button class="chip small" class:on={history} aria-pressed={history} onclick={() => (history = !history)}>{t.history}</button>
      <button class="icon-btn" title={t.fit} aria-label={t.fit} onclick={() => drawing?.fit()}><Icon name="scan" /></button>
      <button class="btn" onclick={tellYoru}><Icon name="message-circle" size={15} />{t.tell}</button>
      <button class="btn primary" onclick={() => ((adding = !adding), (selected = null))}><Icon name="plus" size={15} stroke={2.25} />{t.add}</button>
    </div>
  </header>

  {#if error}<p class="error-text err">{error}</p>{/if}

  <div class="stage">
    <MemoryGraph
      bind:this={drawing}
      {graph}
      {selected}
      {showFacts}
      onselect={pick}
      youName={t.you}
      relationName={(k) => t.relations[k] ?? k}
      kindName={(k) => t.kinds[k] ?? k}
      sectionName={(k) => t.sections[k] ?? k}
      inset={adding || entity ? 372 : 0}
    />

    {#if adding}
      <div class="card add">
        <div class="card-top">
          <b>{t.newTitle}</b>
          <button class="icon-btn small" aria-label={t.close} title={t.close} onclick={() => (adding = false)}><Icon name="x" size={15} /></button>
        </div>
        <input type="text" placeholder={t.newName} bind:value={newName} {@attach (el: HTMLInputElement) => el.focus()} onkeydown={(e) => e.key === "Enter" && addEntity()} />
        <div class="row">
          <select class="compact" bind:value={newKind}>
            {#each KINDS as k}<option value={k}>{t.kinds[k] ?? k}</option>{/each}
          </select>
          <button class="btn small primary" disabled={!newName.trim()} onclick={addEntity}>{t.add}</button>
        </div>
      </div>
    {:else if entity && card}
      <aside class="card inspector">
        <div class="card-top">
          {#if entity.is_user}
            <span class="tag">{t.youTag}</span>
          {:else}
            <select class="compact kind" bind:value={card.kind} aria-label={t.kind}>
              {#each KINDS as k}<option value={k}>{t.kinds[k] ?? k}</option>{/each}
            </select>
          {/if}
          {#if graph.voices.includes(entity.id)}<span class="tag done">{t.voiceKnown}</span>{/if}
          <button class="icon-btn small close" aria-label={t.close} title={t.close} onclick={() => pick(null)}><Icon name="x" size={15} /></button>
        </div>

        <input type="text" class="bare name" bind:value={card.name} aria-label={t.name} onkeydown={fieldKey} />
        {#if entity.label}<p class="faint meta">{entity.label}</p>{/if}
        <input type="text" class="bare" placeholder={t.summary} bind:value={card.summary} aria-label={t.summary} onkeydown={fieldKey} />
        <input type="text" class="bare" placeholder={t.aliases} bind:value={aliases} aria-label={t.aliases} onkeydown={fieldKey} />
        {#if card.kind === "person"}
          <select class="compact gender" bind:value={card.gender} aria-label={t.gender}>
            <option value={null}>{t.genderUnknown}</option>
            <option value="f">{t.genderF}</option>
            <option value="m">{t.genderM}</option>
          </select>
        {/if}
        {#if dirty}
          <div class="row">
            <button class="btn small primary" onclick={saveEntity}>{t.save}</button>
            <button class="btn small quiet" onclick={() => ((card = { ...entity, aliases: [...entity.aliases] }), (aliases = entity.aliases.join(", ")))}>{t.cancel}</button>
          </div>
        {/if}

        <h3>{t.links}</h3>
        <ul class="lines">
          {#each relationsOf(entity.id) as r (r.id)}
            <li>
              <span class="text">{t.relation(nameOf(r.a), t.relations[r.kind] ?? r.kind, nameOf(r.b))}</span>
              <span class="tools"><button class="icon-btn small" aria-label={t.unrelate} title={t.unrelate} onclick={() => act("memory_unrelate", { id: r.id })}><Icon name="x" size={14} /></button></span>
            </li>
          {/each}
        </ul>
        {#if entity.kind === "person"}
          <div class="row relate">
            <span class="faint">{t.isThe(entity.is_user ? t.you(entity.name) : entity.name)}</span>
            <select class="compact" bind:value={relKind}>
              {#each RELATIONS as k}<option value={k}>{t.relations[k] ?? k}</option>{/each}
            </select>
            <span class="faint">{t.of}</span>
            <select class="compact" bind:value={relTo}>
              <option value="">—</option>
              {#each graph.entities.filter((e) => e.id !== entity.id && e.kind === "person") as e (e.id)}<option value={e.id}>{e.is_user ? t.you(e.name) : e.name}</option>{/each}
            </select>
            <button class="icon-btn small outline" disabled={relTo === ""} aria-label={t.add} title={t.add} onclick={() => { const b = relTo as number; relTo = ""; void act("memory_relate", { a: entity.id, kind: relKind, b }); }}><Icon name="plus" size={14} /></button>
          </div>
        {/if}

        <h3>{t.knows}</h3>
        {#each SECTIONS as s}
          {@const facts = factsOf(entity.id).filter((f) => f.section === s)}
          {#if facts.length}
            <p class="section">{t.sections[s]}</p>
            <ul class="lines">
              {#each facts as f (f.id)}
                <li class:closed={f.to !== null} class:lit={lit === f.id}>
                  {#if editing === f.id}
                    <input
                      type="text"
                      class="compact edit"
                      bind:value={editText}
                      {@attach (el: HTMLInputElement) => el.focus()}
                      onblur={() => saveFact(f)}
                      onkeydown={(e) => (e.key === "Enter" ? saveFact(f) : e.key === "Escape" ? (editing = null) : null)}
                    />
                  {:else}
                    <button class="text" disabled={f.to !== null} title={f.to === null ? t.editHint : undefined} onclick={() => ((editing = f.id), (editText = f.text))}>
                      {f.text}
                      {#if f.to !== null}<span class="faint when">{t.wasTrue(day(f.from), day(f.to))}</span>{/if}
                    </button>
                    {#if f.importance >= 0.8}<span class="mark" title={t.important}><Icon name="star" size={13} /></span>{/if}
                    {#if f.private}<span class="mark" title={t.private}><Icon name="lock" size={13} /></span>{/if}
                    <span class="tools">
                      {#if f.to === null}
                        <button class="icon-btn small" class:on={f.importance >= 0.8} title={f.importance >= 0.8 ? t.notImportant : t.makeImportant} aria-label={f.importance >= 0.8 ? t.notImportant : t.makeImportant} onclick={() => flag(f, { importance: f.importance >= 0.8 ? 0.5 : 0.9 })}><Icon name="star" size={14} /></button>
                        <button class="icon-btn small" class:on={f.private} title={f.private ? t.notPrivate : t.makePrivate} aria-label={f.private ? t.notPrivate : t.makePrivate} onclick={() => flag(f, { private: !f.private })}><Icon name="lock" size={14} /></button>
                        <button class="icon-btn small" title={t.noLonger} aria-label={t.noLonger} onclick={() => act("memory_close_fact", { id: f.id })}><Icon name="history" size={14} /></button>
                      {/if}
                      <ConfirmButton icon="trash-2" small label={t.delete} question={t.deleteConfirm} sure={t.deleteSure} onconfirm={() => act("memory_forget_fact", { id: f.id })} />
                    </span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        {/each}
        {#if !factsOf(entity.id).length}<p class="faint meta">{t.noneHint}</p>{/if}

        <div class="new-fact">
          <input type="text" class="compact" placeholder={t.newFact(entity.is_user ? t.you(entity.name) : entity.name)} bind:value={draft.text} onkeydown={(e) => e.key === "Enter" && addFact()} />
          <select class="compact" bind:value={draft.section} aria-label={t.section}>
            {#each SECTIONS as s}<option value={s}>{t.sections[s]}</option>{/each}
          </select>
          <button class="icon-btn small outline" disabled={!draft.text.trim()} aria-label={t.add} title={t.add} onclick={addFact}><Icon name="plus" size={14} /></button>
        </div>

        <div class="foot">
          <button class="btn link" onclick={tellYoru}><Icon name="message-circle" size={14} />{t.tellAbout(entity.is_user ? t.me : entity.name)}</button>
          {#if !entity.is_user}
            <div class="row">
              <span class="faint">{t.sameAs}</span>
              <select class="compact" bind:value={mergeInto}>
                <option value="">—</option>
                {#each graph.entities.filter((e) => e.id !== entity.id) as e (e.id)}<option value={e.id}>{e.is_user ? t.you(e.name) : e.name}</option>{/each}
              </select>
              <button class="btn small" disabled={mergeInto === ""} onclick={() => { const keep = mergeInto as number; mergeInto = ""; selected = keep; void act("memory_merge", { keep, drop: entity.id }); }}>{t.merge}</button>
            </div>
          {/if}
          <div class="row">
            {#if graph.voices.includes(entity.id)}
              <ConfirmButton small label={t.forgetVoice} question={t.forgetVoiceConfirm(entity.name)} sure={t.forgetVoiceSure} onconfirm={() => act("memory_forget_voice", { id: entity.id })} />
            {/if}
            {#if !entity.is_user}
              <ConfirmButton small label={t.remove} question={t.removeConfirm(entity.name)} sure={t.removeSure} onconfirm={() => act("memory_remove_entity", { id: entity.id })} />
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
  .kind {
    min-width: 0;
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
  .inspector input.bare {
    display: block;
    width: calc(100% + 6px);
    color: var(--haze);
  }
  .inspector input.name {
    color: var(--pearl);
    font: 650 var(--fs-head)/1.3 var(--display);
  }
  .meta {
    margin: 0 0 4px;
    font-size: var(--fs-meta);
  }
  .gender {
    margin-top: 6px;
    min-width: 0;
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
  .lines li.closed .text {
    color: var(--dusk);
    text-decoration: line-through;
    cursor: default;
  }
  .when {
    display: block;
    text-decoration: none;
    font-size: var(--fs-small);
  }
  .mark {
    display: grid;
    color: var(--dusk);
  }
  /* A line's tools show on the line pointed at. */
  .tools {
    display: flex;
    align-items: center;
    gap: 0;
    opacity: 0;
    transition: opacity 120ms;
  }
  .lines li:hover .tools,
  .tools:focus-within {
    opacity: 1;
  }
  .lines li:hover .mark {
    display: none;
  }
  .edit {
    flex: 1;
    margin: 2px 0;
  }
  .relate {
    margin-top: 6px;
    font-size: var(--fs-meta);
  }
  .relate select {
    min-width: 0;
  }
  .new-fact {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 10px;
  }
  .new-fact input {
    flex: 1;
    min-width: 0;
  }
  .new-fact select {
    min-width: 0;
    max-width: 112px;
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
  .foot select {
    min-width: 0;
    flex: 1;
  }
</style>
