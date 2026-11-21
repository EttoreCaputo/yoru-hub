<!--
  A model picker: looks like the app's selects, opens a list with a search
  field. It shows the choices it's given (the local model, the brain, the
  catalog's described ones) and, for each service named, every model that
  service offers the key now (`provider_models`, asked once per service
  while the app runs). With `custom`, a name typed in the search can be
  used as it is (a Claude model just out, one pulled into Ollama).
-->
<script module lang="ts">
  import { call, why } from "../api";
  import type { ListedModel } from "../types";

  /** Each service's models, asked once: a failed answer is asked again next time. */
  const asked = new Map<string, Promise<ListedModel[]>>();
  function listOf(provider: string): Promise<ListedModel[]> {
    let list = asked.get(provider);
    if (!list) {
      list = call<ListedModel[]>("provider_models", { provider });
      list.catch(() => asked.delete(provider));
      asked.set(provider, list);
    }
    return list;
  }

  export type Choice = { value: string; label: string; note?: string; group?: string };
</script>

<script lang="ts">
  import { tick } from "svelte";
  import { i18n } from "../i18n/index.svelte";
  import Icon from "./Icon.svelte";

  let {
    value,
    choices = [],
    providers = [],
    need = "chat",
    bare = false,
    custom = false,
    compact = false,
    accept = () => true,
    label,
    onchange,
  }: {
    value: string;
    /** Shown first, in their groups. */
    choices?: Choice[];
    /** Services whose every model is listed after the choices. */
    providers?: { id: string; name: string }[];
    /** Which of their models: the ones that write, or the ones that hear. */
    need?: "chat" | "stt";
    /** The value is the model alone (`gemini-3.6-flash`), not `google:gemini-3.6-flash`. */
    bare?: boolean;
    /** A typed name can be used as it is. */
    custom?: boolean;
    /** Small and without the field's fill: for a toolbar. */
    compact?: boolean;
    /** Which of the services' models fit, beyond `need`. */
    accept?: (m: ListedModel) => boolean;
    label: string;
    onchange: (value: string) => void;
  } = $props();

  const t = $derived(i18n.t.picker);

  let open = $state(false);
  let query = $state("");
  let active = $state(0);
  let live = $state<Record<string, ListedModel[]>>({});
  let errors = $state<Record<string, string>>({});
  let loading = $state(0);

  let trigger: HTMLButtonElement | undefined = $state();
  let search: HTMLInputElement | undefined = $state();
  let list: HTMLElement | undefined = $state();
  let place = $state({ left: 0, top: 0, width: 0, maxHeight: 360, up: false });

  async function load() {
    for (const p of providers) {
      if (live[p.id] || errors[p.id]) continue;
      loading++;
      try {
        live = { ...live, [p.id]: await listOf(p.id) };
      } catch (e) {
        errors = { ...errors, [p.id]: why(e) };
      } finally {
        loading--;
      }
    }
  }

  /** Every choice: the given ones, then each service's models not among them. */
  const all = $derived.by<Choice[]>(() => {
    const out = [...choices];
    const seen = new Set(out.map((c) => c.value));
    for (const p of providers) {
      for (const m of live[p.id] ?? []) {
        const v = bare ? m.model : m.id;
        if (!(need === "chat" ? m.chat : m.stt) || !accept(m) || seen.has(v)) continue;
        seen.add(v);
        out.push({ value: v, label: m.name !== m.model ? m.name : m.model, note: m.note || (m.name !== m.model ? m.model : ""), group: t.all(p.name) });
      }
    }
    return out;
  });

  const words = $derived(query.trim().toLowerCase().split(/\s+/).filter(Boolean));
  const shown = $derived(all.filter((c) => words.every((w) => `${c.label} ${c.value} ${c.note ?? ""} ${c.group ?? ""}`.toLowerCase().includes(w))));
  const typed = $derived(custom && query.trim() && !all.some((c) => c.value === query.trim()) ? query.trim() : "");
  const current = $derived(all.find((c) => c.value === value));

  function place_() {
    if (!trigger) return;
    const r = trigger.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - 12;
    const above = r.top - 12;
    const up = below < 260 && above > below;
    const width = Math.min(Math.max(r.width, 320), window.innerWidth - 24);
    const left = Math.min(Math.max(12, r.left), window.innerWidth - width - 12);
    place = { left, top: up ? r.top - 6 : r.bottom + 6, width, maxHeight: Math.min(380, (up ? above : below) - 6), up };
  }

  async function show() {
    if (open) return close();
    query = "";
    place_();
    open = true;
    void load();
    await tick();
    active = Math.max(0, shown.findIndex((c) => c.value === value));
    search?.focus();
    scrollToActive();
  }
  function close(refocus = true) {
    open = false;
    if (refocus) trigger?.focus();
  }
  function pick(v: string) {
    if (v !== value) onchange(v);
    close();
  }

  function scrollToActive() {
    void tick().then(() => list?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" }));
  }
  function keys(e: KeyboardEvent) {
    const n = shown.length + (typed ? 1 : 0);
    if (e.key === "ArrowDown") active = Math.min(n - 1, active + 1);
    else if (e.key === "ArrowUp") active = Math.max(0, active - 1);
    else if (e.key === "Enter") {
      if (active < shown.length) pick(shown[active].value);
      else if (typed) pick(typed);
    } else if (e.key === "Escape") close();
    else if (e.key === "Tab") close(false);
    else return;
    e.preventDefault();
    e.stopPropagation();
    scrollToActive();
  }
  $effect(() => {
    void words.length;
    active = 0;
  });

  function outside(e: PointerEvent) {
    const target = e.target as Node;
    if (open && !trigger?.contains(target) && !list?.parentElement?.contains(target)) close(false);
  }
</script>

<svelte:window onpointerdown={outside} onresize={() => open && place_()} onscrollcapture={(e) => open && !list?.parentElement?.contains(e.target as Node) && place_()} />

<button bind:this={trigger} type="button" class="trigger" class:open class:compact aria-haspopup="listbox" aria-expanded={open} aria-label={label} title={current?.note || value} onclick={show}>
  <span class="now">{current?.label ?? value}</span>
</button>

{#if open}
  <div class="pop" class:up={place.up} style:left="{place.left}px" style:top="{place.top}px" style:width="{place.width}px" role="dialog" aria-label={label}>
    <label class="find">
      <Icon name="search" size={15} />
      <input bind:this={search} type="text" bind:value={query} placeholder={t.search} spellcheck="false" autocomplete="off" onkeydown={keys} aria-label={t.search} />
      {#if query}<button type="button" class="icon-btn small" aria-label="×" onclick={() => ((query = ""), search?.focus())}><Icon name="x" size={14} /></button>{/if}
    </label>
    <ul bind:this={list} role="listbox" aria-label={label} style:max-height="{place.maxHeight - 52}px">
      {#each shown as c, i (c.value)}
        {#if i === 0 || c.group !== shown[i - 1].group}
          {#if c.group}<li class="group" role="presentation">{c.group}</li>{/if}
        {/if}
        <li role="option" aria-selected={c.value === value} data-i={i} class:active={i === active} onpointermove={() => (active = i)} onclick={() => pick(c.value)} onkeydown={() => {}}>
          <span class="mark">{#if c.value === value}<Icon name="check" size={15} stroke={2.2} />{/if}</span>
          <span class="words">
            <b>{c.label}</b>
            {#if c.note}<span class="note">{c.note}</span>{/if}
          </span>
        </li>
      {/each}
      {#if typed}
        <li role="option" aria-selected="false" data-i={shown.length} class:active={active === shown.length} onpointermove={() => (active = shown.length)} onclick={() => pick(typed)} onkeydown={() => {}}>
          <span class="mark"></span>
          <span class="words"><b>{t.use(typed)}</b></span>
        </li>
      {/if}
      {#if !shown.length && !typed && !loading}<li class="empty" role="presentation">{t.none}</li>{/if}
      {#if loading}<li class="empty" role="presentation"><span class="spin"></span>{t.loading}</li>{/if}
      {#each Object.values(errors) as e (e)}<li class="empty error-text" role="presentation">{t.unreachable(e)}</li>{/each}
    </ul>
  </div>
{/if}

<style>
  /* Drawn like the app's selects (app.css): rim, fill, chevron. */
  .trigger {
    display: inline-flex;
    align-items: center;
    width: auto;
    min-width: 160px;
    max-width: 100%;
    min-height: 36px;
    padding: 8px 36px 8px 12px;
    border-radius: var(--r);
    border: 1px solid var(--rim-2);
    background: var(--field) var(--chevron) no-repeat right 12px center / 16px;
    color: var(--pearl);
    font: 400 var(--fs-body)/1.4 var(--sans);
    text-align: left;
    cursor: pointer;
    transition: border-color 160ms;
  }
  .trigger:hover,
  .trigger.open,
  .trigger:focus-visible {
    outline: none;
    border-color: var(--pearl-deep);
  }
  .trigger.compact {
    min-width: 0;
    min-height: 30px;
    padding: 4px 30px 4px 10px;
    border-color: transparent;
    background-color: transparent;
    background-position: right 8px center;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .trigger.compact:hover,
  .trigger.compact.open {
    background-color: var(--hover);
    border-color: transparent;
    color: var(--pearl);
  }
  .now {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pop {
    position: fixed;
    z-index: 50;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--rim-2);
    border-radius: var(--r-lg);
    background: var(--ink);
    box-shadow: var(--shadow), var(--lift);
    overflow: hidden;
    animation: drop 160ms cubic-bezier(0.2, 0.8, 0.2, 1);
    transform-origin: top center;
  }
  .pop.up {
    transform: translateY(-100%);
    transform-origin: bottom center;
    animation-name: lift;
  }
  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.98);
    }
  }
  @keyframes lift {
    from {
      opacity: 0;
      transform: translateY(calc(-100% + 4px)) scale(0.98);
    }
  }
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 14px;
    height: 46px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--rim);
    color: var(--haze);
  }
  .find input {
    flex: 1;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--pearl);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 6px;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  li[role="option"] {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 7px 10px 7px 6px;
    border-radius: var(--r-sm);
    cursor: pointer;
  }
  li.active {
    background: var(--hover);
  }
  .mark {
    flex: 0 0 18px;
    display: grid;
    place-items: center;
    height: 20px;
    color: var(--pearl);
  }
  .words {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .words b {
    font: 500 var(--fs-body)/1.4 var(--sans);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    color: var(--haze);
    font-size: var(--fs-small);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .group {
    padding: 10px 10px 4px 32px;
    color: var(--dusk);
    font: 600 var(--fs-small)/1.3 var(--sans);
    letter-spacing: 0.02em;
  }
  .group:first-child {
    padding-top: 4px;
  }
  .empty {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 10px 10px 32px;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .spin {
    width: 12px;
    height: 12px;
    margin-left: -20px;
    border: 1.5px solid var(--rim-2);
    border-top-color: var(--haze);
    border-radius: 50%;
    animation: spin 800ms linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
