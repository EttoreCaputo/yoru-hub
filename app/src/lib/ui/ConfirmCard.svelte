<!--
  A card waiting for Allow or Deny: something Yoru is about to do (send a
  mail, create an event). The same card is on the board; whichever answers
  first wins.
-->
<script lang="ts">
  import { why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Confirmation } from "../types";
  import { clock } from "./when";

  let { card }: { card: Confirmation } = $props();
  const t = $derived(i18n.t.card);
  const left = $derived(Math.max(0, (card.expires - store.now) / 1000));
  let open = $state(false);
  let busy = $state(false);
  let error = $state("");

  async function answer(allow: boolean) {
    busy = true;
    error = "";
    try {
      await store.confirm(card.id, allow);
    } catch (e) {
      error = why(e);
      busy = false;
    }
  }
</script>

<article class="card">
  <header>
    <span class="dot wait"></span>
    <span class="who">{t.yoruAsks}</span>
    <span class="tool mono">{card.tool}</span>
    <span class="left faint">{t.expires(clock(left))}</span>
  </header>
  <p class="what">{card.text}</p>
  {#if card.detail}
    <button class="btn quiet small more" onclick={() => (open = !open)} aria-expanded={open}>{t.details}</button>
    {#if open}<pre class="detail mono">{card.detail}</pre>{/if}
  {/if}
  <div class="row actions">
    <button class="btn primary" disabled={busy} onclick={() => answer(true)}>{t.allow}</button>
    <button class="btn" disabled={busy} onclick={() => answer(false)}>{t.deny}</button>
  </div>
  {#if error}<p class="error-text">{error}</p>{/if}
</article>

<style>
  .card {
    padding: 14px 16px;
    border-radius: 16px;
    background: color-mix(in srgb, var(--wait) 6%, var(--ink));
    border: 1px solid color-mix(in srgb, var(--wait) 28%, transparent);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .who {
    font-weight: 600;
  }
  .tool {
    color: var(--haze);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .left {
    margin-left: auto;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .what {
    margin: 8px 0 4px;
    font-size: 14px;
    user-select: text;
  }
  .more {
    padding: 0;
    height: auto;
  }
  .detail {
    margin: 6px 0 0;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--night);
    color: var(--haze);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 180px;
    overflow: auto;
    user-select: text;
  }
  .actions {
    margin-top: 12px;
  }
</style>
