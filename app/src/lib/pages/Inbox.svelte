<!-- Every notification, by what's unread and where it came from. -->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import NotificationRow from "../ui/NotificationRow.svelte";

  const t = $derived(i18n.t.inbox);
  let show = $state<"unread" | "all">(store.unread ? "unread" : "all");
  let source = $state<string | null>(null);

  const sources = $derived([...new Set(store.inbox.map((n) => n.source))].sort());
  const list = $derived(store.inbox.filter((n) => (show === "all" || !n.read) && (!source || n.source === source)));

  async function clear() {
    if (confirm(t.clearConfirm)) await store.clearInbox();
  }
</script>

<div class="page">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  <div class="bar">
    <div class="seg" role="tablist">
      <button role="tab" aria-selected={show === "unread"} class:on={show === "unread"} onclick={() => (show = "unread")}>
        {t.unread}{#if store.unread}<span class="n">{store.unread}</span>{/if}
      </button>
      <button role="tab" aria-selected={show === "all"} class:on={show === "all"} onclick={() => (show = "all")}>{t.all}</button>
    </div>
    <div class="sources row">
      <button class="chip" class:on={!source} onclick={() => (source = null)}>{t.everySource}</button>
      {#each sources as s (s)}
        <button class="chip" class:on={source === s} onclick={() => (source = source === s ? null : s)}>{s}</button>
      {/each}
    </div>
    <div class="row end">
      <button class="btn quiet small" disabled={!store.unread} onclick={() => store.readAll()}>{t.readAll}</button>
      <button class="btn quiet small" disabled={!store.inbox.length} onclick={clear}>{t.clear}</button>
    </div>
  </div>

  {#if list.length}
    <div class="list">
      {#each list as n (n.id)}<NotificationRow {n} />{/each}
    </div>
  {:else if show === "unread" && store.inbox.length}
    <div class="empty"><b>{t.emptyUnread}</b><p>{t.emptyUnreadHint}</p></div>
  {:else}
    <div class="empty">
      <b>{t.emptyAll}</b>
      <p>{t.emptyAllHint}</p>
      <button class="btn" style="margin-top:16px" onclick={() => store.go("routines")}>{i18n.t.nav.routines}</button>
    </div>
  {/if}
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--rim);
    margin-bottom: 4px;
  }
  .seg {
    display: flex;
    padding: 3px;
    border-radius: 999px;
    background: var(--ink-2);
    border: 1px solid var(--rim);
  }
  .seg button {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 12px;
    border: 0;
    border-radius: 999px;
    background: none;
    color: var(--haze);
    font: 550 12.5px/1 var(--sans);
    cursor: pointer;
  }
  .seg button.on {
    background: var(--ink-3);
    color: var(--pearl);
  }
  .n {
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--pearl);
    color: var(--eye);
    font: 700 10.5px/18px var(--sans);
    text-align: center;
  }
  .sources {
    flex: 1;
    gap: 4px;
  }
  .chip {
    height: 26px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid transparent;
    background: none;
    color: var(--dusk);
    font: 500 12.5px/1 var(--sans);
    cursor: pointer;
  }
  .chip:hover {
    color: var(--pearl);
  }
  .chip.on {
    color: var(--pearl);
    border-color: var(--rim-2);
  }
  .end {
    gap: 2px;
  }
</style>
