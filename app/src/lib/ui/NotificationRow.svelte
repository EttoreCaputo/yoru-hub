<!--
  One notification: where it's from, what it says, and what you can do with
  it. `compact` is Today's: no actions but opening it.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Notification } from "../types";
  import { ago } from "./when";

  let { n, compact = false }: { n: Notification; compact?: boolean } = $props();
  const t = $derived(i18n.t.inbox);
  const watcher = $derived(n.by ? store.watchers.find((w) => w.watcher.id === n.by)?.watcher.name : undefined);
  let note = $state("");
  let busy = $state(false);

  async function run(job: () => Promise<unknown>) {
    busy = true;
    note = "";
    try {
      await job();
    } catch (e) {
      note = why(e);
    } finally {
      busy = false;
    }
  }

  /** Into the memory's "Don't tell me about", through Yoru so it words it well; dismissed meanwhile. */
  const mute = () =>
    run(async () => {
      await store.dismiss(n.id);
      if (store.info?.ready) await call("ask", { text: t.muteSay(n.source, n.title) });
      else {
        const memory = await call<string>("memory");
        await call("set_memory", { text: addQuiet(memory, `${n.source}: ${n.title}`) });
      }
      note = t.muted;
    });

  function addQuiet(memory: string, line: string) {
    const head = "## Don't tell me about";
    const at = memory.indexOf(head);
    if (at < 0) return `${memory.trimEnd()}\n\n${head}\n\n- ${line}\n`;
    const end = at + head.length;
    return `${memory.slice(0, end)}\n\n- ${line}${memory.slice(end).replace(/^\n+/, "\n")}`;
  }

  const opener = () => run(() => store.open(n.id));
</script>

<article class="note" class:unread={!n.read} class:compact>
  <span class="mark {n.urgency}" aria-label={t.urgency[n.urgency]} title={t.urgency[n.urgency]}></span>
  <div class="body">
    <header>
      <span class="source">{n.source}</span>
      {#if watcher && watcher !== n.source}<span class="faint">{t.from(watcher)}</span>{/if}
      <time class="faint" datetime={new Date(n.at).toISOString()}>{ago(n.at, store.now)}</time>
    </header>
    {#if compact}
      <button class="title link" onclick={opener}>{n.title}</button>
    {:else}
      <h3 class="title">{n.title}</h3>
    {/if}
    {#if n.text}<p class="text">{n.text}</p>{/if}
    {#if !compact}
      <div class="row actions">
        {#if n.url}<button class="btn small" disabled={busy} onclick={opener}>{t.open}</button>{/if}
        {#if !n.read}<button class="btn small quiet" disabled={busy} onclick={() => run(() => store.read(n.id))}>{t.read}</button>{/if}
        <button class="btn small quiet" disabled={busy} onclick={() => run(() => store.dismiss(n.id))}>{t.dismiss}</button>
        <button class="btn small quiet" disabled={busy} onclick={mute}>{t.mute}</button>
      </div>
    {/if}
    {#if note}<p class="note-line dim">{note}</p>{/if}
  </div>
</article>

<style>
  .note {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr);
    gap: 10px;
    padding: 14px 4px;
  }
  .note + :global(.note) {
    border-top: 1px solid var(--rim);
  }
  .compact {
    padding: 10px 0;
  }
  /* The urgency, as a small moon: full when it's urgent. */
  .mark {
    width: 9px;
    height: 9px;
    margin-top: 6px;
    border-radius: 50%;
    border: 1.5px solid var(--dusk);
  }
  .mark.normal {
    border-color: var(--pearl-mid);
  }
  .mark.high {
    border-color: var(--wait);
    background: linear-gradient(90deg, var(--wait) 50%, transparent 50%);
  }
  .mark.urgent {
    border-color: var(--wrong);
    background: var(--wrong);
    box-shadow: 0 0 8px var(--wrong);
  }
  .body {
    min-width: 0;
  }
  header {
    display: flex;
    gap: 8px;
    align-items: baseline;
    font-size: 12px;
  }
  .source {
    color: var(--haze);
    font-weight: 600;
  }
  time {
    margin-left: auto;
    white-space: nowrap;
  }
  .title {
    display: block;
    margin: 2px 0 0;
    font: 500 14.5px/1.35 var(--sans);
    color: var(--haze);
    text-align: left;
  }
  .unread .title {
    color: var(--pearl);
    font-weight: 600;
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
    text-decoration-color: var(--rim-2);
    text-underline-offset: 3px;
  }
  .text {
    margin-top: 3px;
    color: var(--haze);
    font-size: 13.5px;
    user-select: text;
  }
  .compact .text {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .actions {
    margin-top: 10px;
    gap: 4px;
  }
  .note-line {
    margin-top: 6px;
    font-size: 12.5px;
  }
</style>
