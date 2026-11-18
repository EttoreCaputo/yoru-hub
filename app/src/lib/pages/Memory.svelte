<!--
  What Yoru knows, as a graph (docs/memory.md): the people and things in
  your life, what is true of each, and what used to be. Yoru writes it as you
  talk and while it rests; here you correct it. Read again when you come
  back, since Yoru may have learnt something meanwhile.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import Toggle from "../ui/Toggle.svelte";
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
  let history = $state(false);
  let selected = $state<number | null>(null);

  async function load() {
    try {
      graph = await call<Graph>("memory_graph");
      if (selected === null || !graph.entities.some((e) => e.id === selected)) selected = user()?.id ?? null;
    } catch (e) {
      error = why(e);
    }
  }
  $effect(() => {
    void load();
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
  const others = $derived(graph.entities.filter((e) => !e.is_user).sort((a, b) => (a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === "person" ? -1 : b.kind === "person" ? 1 : a.kind.localeCompare(b.kind))));
  const aboutOf = (f: Fact) => f.about ?? user()?.id ?? null;
  const factsOf = (id: number) => graph.facts.filter((f) => aboutOf(f) === id && (history || f.to === null));
  const openCount = (id: number) => graph.facts.filter((f) => aboutOf(f) === id && f.to === null).length;
  const nameOf = (id: number) => graph.entities.find((e) => e.id === id)?.name ?? `#${id}`;
  const relationsOf = (id: number) => graph.relations.filter((r) => r.a === id || r.b === id);
  const day = (ms: number) => i18n.date(ms, { day: "numeric", month: "short", year: "numeric" });

  // A new fact about the one selected.
  let draft = $state({ text: "", section: "about" as string, important: false, private: false });
  async function addFact() {
    if (!draft.text.trim() || selected === null) return;
    await act("memory_put_fact", { fact: { id: 0, about: selected, section: draft.section, text: draft.text.trim(), from: 0, to: null, importance: draft.important ? 0.9 : 0.5, private: draft.private, episode: null } });
    draft = { ...draft, text: "", important: false, private: false };
  }

  // A fact being corrected.
  let editing = $state<number | null>(null);
  let editText = $state("");
  async function saveFact(f: Fact) {
    if (editText.trim() && editText.trim() !== f.text) await act("memory_put_fact", { fact: { ...f, text: editText.trim() } });
    editing = null;
  }
  const flag = (f: Fact, change: Partial<Fact>) => act("memory_put_fact", { fact: { ...f, ...change } });

  // Someone new.
  let newName = $state("");
  let newKind = $state("person");
  async function addEntity() {
    if (!newName.trim()) return;
    error = "";
    try {
      selected = await call<number>("memory_add_entity", { name: newName.trim(), kind: newKind });
      newName = "";
    } catch (e) {
      error = why(e);
    }
    await load();
  }

  // The one selected, being edited.
  let card = $state<Entity | null>(null);
  $effect(() => {
    card = entity ? { ...entity, aliases: [...entity.aliases] } : null;
  });
  const cardDirty = $derived(!!card && !!entity && (card.name !== entity.name || card.summary !== entity.summary || card.kind !== entity.kind || card.gender !== entity.gender || card.aliases.join(", ") !== entity.aliases.join(", ")));
  let aliases = $state("");
  $effect(() => {
    aliases = entity?.aliases.join(", ") ?? "";
  });
  async function saveEntity() {
    if (!card) return;
    const list = aliases.split(",").map((a) => a.trim()).filter(Boolean);
    await act("memory_put_entity", { entity: { ...card, aliases: list, placeholder: card.placeholder && card.name === entity?.name } });
  }

  // List or graph, remembered on this computer.
  const read = (key: string, fallback: string) => {
    try {
      return localStorage.getItem(key) ?? fallback;
    } catch {
      return fallback;
    }
  };
  let mode = $state<"list" | "graph">(read("memory.view", "graph") === "list" ? "list" : "graph");
  let showFacts = $state(read("memory.facts", "1") === "1");
  $effect(() => {
    try {
      localStorage.setItem("memory.view", mode);
      localStorage.setItem("memory.facts", showFacts ? "1" : "0");
    } catch {
      // Not kept: fine.
    }
  });
  let drawing = $state<MemoryGraph>();

  let mergeInto = $state<number | "">("");
  let relKind = $state("partner");
  let relTo = $state<number | "">("");
</script>

<div class="page memory">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    <div class="actions">
      <div class="seg" role="tablist">
        <button role="tab" aria-selected={mode === "graph"} class:on={mode === "graph"} onclick={() => (mode = "graph")}>{t.graph}</button>
        <button role="tab" aria-selected={mode === "list"} class:on={mode === "list"} onclick={() => (mode = "list")}>{t.list}</button>
      </div>
      <Toggle checked={history} label={t.history} onchange={(on) => (history = on)} />
      <span class="dim small">{t.history}</span>
    </div>
  </div>

  {#if error}<p class="error-text">{error}</p>{/if}

  {#snippet addNew()}
      <div class="add-entity">
        <input type="text" placeholder={t.newName} bind:value={newName} onkeydown={(e) => e.key === "Enter" && addEntity()} />
        <select bind:value={newKind}>
          {#each KINDS as k}<option value={k}>{t.kinds[k] ?? k}</option>{/each}
        </select>
        <button class="btn small" disabled={!newName.trim()} onclick={addEntity}>{t.add}</button>
      </div>
  {/snippet}

  {#snippet detail()}
    {#if entity && card}
      <section class="detail">
        <div class="panel">
          <div class="card-head">
            <input type="text" class="name" bind:value={card.name} aria-label={t.name} />
            {#if !entity.is_user}
              <select bind:value={card.kind}>
                {#each KINDS as k}<option value={k}>{t.kinds[k] ?? k}</option>{/each}
              </select>
            {/if}
            {#if card.kind === "person"}
              <select bind:value={card.gender}>
                <option value={null}>{t.genderUnknown}</option>
                <option value="f">{t.genderF}</option>
                <option value="m">{t.genderM}</option>
              </select>
            {/if}
          </div>
          {#if entity.label}<p class="dim small">{entity.label}</p>{/if}
          <input type="text" class="summary" placeholder={t.summary} bind:value={card.summary} />
          <input type="text" class="summary" placeholder={t.aliases} bind:value={aliases} />
          <div class="row actions">
            <button class="btn small primary" disabled={!cardDirty && aliases === entity.aliases.join(", ")} onclick={saveEntity}>{t.save}</button>
            {#if graph.voices.includes(entity.id)}
              <ConfirmButton small label={t.forgetVoice} question={t.forgetVoiceConfirm(entity.name)} sure={t.forgetVoiceSure} onconfirm={() => act("memory_forget_voice", { id: entity.id })} />
            {/if}
            {#if !entity.is_user}
              <ConfirmButton small label={t.remove} question={t.removeConfirm(entity.name)} sure={t.removeSure} onconfirm={() => act("memory_remove_entity", { id: entity.id })} />
            {/if}
          </div>

          {#if !entity.is_user}
            <div class="row merge">
              <span class="dim small">{t.sameAs}</span>
              <select bind:value={mergeInto}>
                <option value="">—</option>
                {#each graph.entities.filter((e) => e.id !== entity.id) as e (e.id)}<option value={e.id}>{e.is_user ? t.you(e.name) : e.name}</option>{/each}
              </select>
              <button class="btn small" disabled={mergeInto === ""} onclick={() => { const keep = mergeInto as number; mergeInto = ""; selected = keep; void act("memory_merge", { keep, drop: entity.id }); }}>{t.merge}</button>
            </div>
          {/if}

          <div class="relations">
            {#each relationsOf(entity.id) as r (r.id)}
              <div class="rel small">
                <span>{t.relation(nameOf(r.a), t.relations[r.kind] ?? r.kind, nameOf(r.b))}</span>
                <button class="btn quiet small" onclick={() => act("memory_unrelate", { id: r.id })}>{t.unrelate}</button>
              </div>
            {/each}
            <div class="row">
              <span class="dim small">{t.isThe(entity.name)}</span>
              <select bind:value={relKind}>
                {#each RELATIONS as k}<option value={k}>{t.relations[k] ?? k}</option>{/each}
              </select>
              <span class="dim small">{t.of}</span>
              <select bind:value={relTo}>
                <option value="">—</option>
                {#each graph.entities.filter((e) => e.id !== entity.id && e.kind === "person") as e (e.id)}<option value={e.id}>{e.is_user ? t.you(e.name) : e.name}</option>{/each}
              </select>
              <button class="btn small" disabled={relTo === ""} onclick={() => { const b = relTo as number; relTo = ""; void act("memory_relate", { a: entity.id, kind: relKind, b }); }}>{t.add}</button>
            </div>
          </div>
        </div>

        {#each SECTIONS as s}
          {@const facts = factsOf(entity.id).filter((f) => f.section === s)}
          {#if facts.length}
            <div class="panel">
              <div class="section-title">{t.sections[s]}</div>
              <ul class="facts">
                {#each facts as f (f.id)}
                  <li class:closed={f.to !== null}>
                    {#if editing === f.id}
                      <input type="text" class="edit" bind:value={editText} onkeydown={(e) => (e.key === "Enter" ? saveFact(f) : e.key === "Escape" ? (editing = null) : null)} />
                      <button class="btn small primary" onclick={() => saveFact(f)}>{t.save}</button>
                    {:else}
                      <button class="text" disabled={f.to !== null} onclick={() => { editing = f.id; editText = f.text; }}>{f.text}</button>
                      {#if f.private}<span class="tag">{t.private}</span>{/if}
                      {#if f.importance >= 0.8}<span class="tag">{t.important}</span>{/if}
                      {#if f.to !== null}
                        <span class="faint small">{t.wasTrue(day(f.from), day(f.to))}</span>
                      {:else}
                        <span class="spacer"></span>
                        <button class="btn quiet small" onclick={() => flag(f, { importance: f.importance >= 0.8 ? 0.5 : 0.9 })}>{f.importance >= 0.8 ? t.notImportant : t.makeImportant}</button>
                        <button class="btn quiet small" onclick={() => flag(f, { private: !f.private })}>{f.private ? t.notPrivate : t.makePrivate}</button>
                        <button class="btn quiet small" onclick={() => act("memory_close_fact", { id: f.id })}>{t.noLonger}</button>
                      {/if}
                      <ConfirmButton small label={t.delete} question={t.deleteConfirm} sure={t.deleteSure} onconfirm={() => act("memory_forget_fact", { id: f.id })} />
                    {/if}
                  </li>
                {/each}
              </ul>
            </div>
          {/if}
        {/each}

        {#if !factsOf(entity.id).length}
          <div class="panel empty"><b>{t.none}</b><p>{t.noneHint}</p></div>
        {/if}

        <div class="panel add-fact">
          <input type="text" placeholder={t.newFact(entity.is_user ? t.you(entity.name) : entity.name)} bind:value={draft.text} onkeydown={(e) => e.key === "Enter" && addFact()} />
          <select bind:value={draft.section}>
            {#each SECTIONS as s}<option value={s}>{t.sections[s]}</option>{/each}
          </select>
          <label class="small"><input type="checkbox" bind:checked={draft.important} /> {t.important}</label>
          <label class="small"><input type="checkbox" bind:checked={draft.private} /> {t.private}</label>
          <button class="btn small primary" disabled={!draft.text.trim()} onclick={addFact}>{t.add}</button>
        </div>
      </section>
    {/if}
  {/snippet}

  {#if mode === "graph"}
    <div class="graph-bar row">
      <Toggle checked={showFacts} label={t.showFacts} onchange={(on) => (showFacts = on)} />
      <span class="dim small">{t.showFacts}</span>
      <span class="spacer-flex"></span>
      <button class="btn quiet small" onclick={() => drawing?.fit()}>{t.fit}</button>
    </div>
    <MemoryGraph
      bind:this={drawing}
      {graph}
      {selected}
      {showFacts}
      onselect={(id) => (selected = id)}
      youName={t.you}
      relationName={(k) => t.relations[k] ?? k}
      kindName={(k) => t.kinds[k] ?? k}
      sectionName={(k) => t.sections[k] ?? k}
    />
    <div class="panel add-under">{@render addNew()}</div>
    <div class="under">{@render detail()}</div>
  {:else}
    <div class="split">
    <aside class="panel people">
      <div class="section-title">{t.people}</div>
      <ul>
        {#each [user(), ...others].filter((e) => !!e) as e (e!.id)}
          {@const x = e!}
          <li>
            <button class="nav-row who" class:on={x.id === selected} onclick={() => (selected = x.id)}>
              <b>{x.is_user ? t.you(x.name) : x.name}</b>
              <span class="faint small">
                {x.label ?? (x.kind === "person" ? "" : t.kinds[x.kind] ?? x.kind)}
                {#if x.placeholder}· {t.noName}{/if}
                {#if graph.voices.includes(x.id)}· {t.voiceKnown}{/if}
              </span>
              <span class="count faint">{openCount(x.id)}</span>
            </button>
          </li>
        {/each}
      </ul>
      {@render addNew()}
    </aside>
      {@render detail()}
    </div>
  {/if}
</div>

<style>
  .small {
    font-size: var(--fs-meta);
  }
  .split {
    display: grid;
    grid-template-columns: minmax(220px, 280px) 1fr;
    gap: 16px;
    align-items: start;
  }
  @media (max-width: 760px) {
    .split {
      grid-template-columns: 1fr;
    }
  }
  .people ul,
  .facts {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .who {
    display: grid;
    grid-template-columns: 1fr auto;
  }
  .who b {
    font-weight: 600;
  }
  .who .small {
    grid-column: 1;
  }
  .who .count {
    grid-row: 1 / span 2;
    grid-column: 2;
    align-self: center;
    font-size: var(--fs-meta);
  }
  .add-entity,
  .add-fact,
  .card-head {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 10px;
  }
  .add-entity input,
  .add-fact > input {
    flex: 1;
    min-width: 140px;
  }
  .card-head {
    margin-top: 0;
  }
  .card-head .name {
    flex: 1;
    font: 600 var(--fs-head)/1.3 var(--sans);
  }
  .summary {
    width: 100%;
    margin-top: 8px;
  }
  .actions,
  .merge {
    margin-top: 10px;
    gap: 6px;
    flex-wrap: wrap;
  }
  .relations {
    margin-top: 12px;
    display: grid;
    gap: 6px;
  }
  .relations .row {
    gap: 6px;
    flex-wrap: wrap;
  }
  .rel {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .facts li {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    padding: 6px 0;
  }
  .facts li + li {
    border-top: 1px solid var(--rim);
  }
  .facts .text {
    flex: 1;
    min-width: 200px;
    text-align: left;
    border: 0;
    background: transparent;
    color: var(--pearl);
    font: inherit;
    font-size: var(--fs-body);
    padding: 2px 0;
    cursor: text;
  }
  .facts li.closed .text {
    color: var(--haze);
    text-decoration: line-through;
    cursor: default;
  }
  .facts .edit {
    flex: 1;
  }
  .spacer {
    flex: 0;
  }
  .graph-bar {
    gap: 8px;
    margin-bottom: 10px;
  }
  .spacer-flex {
    flex: 1;
  }
  .add-under {
    margin-top: 12px;
  }
  .add-under .add-entity {
    margin-top: 0;
  }
  .under {
    margin-top: 12px;
  }
</style>
