<!-- Every notification, by what's unread and where it came from. -->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import NotificationRow from "../ui/NotificationRow.svelte";

  const t = $derived(i18n.t.inbox);
  let show = $state<"unread" | "all">(store.unread ? "unread" : "all");
  let source = $state<string | null>(null);

  const sources = $derived([...new Set(store.inbox.map((n) => n.source))].sort());
  const list = $derived(store.inbox.filter((n) => (show === "all" || !n.read) && (!source || n.source === source)));
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
    <div class="chips sources">
      <button class="chip" class:on={!source} onclick={() => (source = null)}>{t.everySource}</button>
      {#each sources as s (s)}
        <button class="chip" class:on={source === s} onclick={() => (source = source === s ? null : s)}>{s}</button>
      {/each}
    </div>
    <div class="row end">
      <button class="btn quiet small" disabled={!store.unread} onclick={() => store.readAll()}>{t.readAll}</button>
      <ConfirmButton small label={t.clear} question={t.clearConfirm} sure={t.clearSure} disabled={!store.inbox.length} onconfirm={() => store.clearInbox()} />
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
    margin-bottom: 4px;
  }
  .n {
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--pearl);
    color: var(--eye);
    font: 700 var(--fs-small)/18px var(--sans);
    text-align: center;
  }
  /* The sources on a line of their own, under the views and the buttons. */
  .sources {
    order: 3;
    flex-basis: 100%;
  }
  .end {
    gap: 2px;
    margin-left: auto;
  }
</style>
