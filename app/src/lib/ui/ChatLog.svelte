<!--
  The conversation with Yoru, typed or spoken: its tools folded under each
  answer, what you're saying while you say it, his thinking dots, and the
  cards he's waiting on right where he stopped. A conversation's tab shows
  it: the one Yoru is in live, an earlier one as it was (`lines`).
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ChatLine } from "../types";
  import ConfirmCard from "./ConfirmCard.svelte";
  import { ago } from "./when";

  let { lines, live = true }: { lines?: ChatLine[]; live?: boolean } = $props();
  const t = $derived(i18n.t.conversation);
  const shown = $derived(lines ?? store.chat);
  const failed = (l: ChatLine) => l.from === "yoru" && l.text.startsWith("(error:");
</script>

<ol class="log">
  {#each shown as l, i (i)}
    <li class="line {l.from}" class:failed={failed(l)}>
      <div class="meta">
        <span class="who">{l.from === "you" ? t.you : t.yoru}</span>
        {#if l.channel === "voice"}<span class="tag">{t.voice}</span>{/if}
        <time class="faint">{ago(l.at, store.now)}</time>
      </div>
      <p class="text">{l.text}</p>
      {#if l.actions?.length}
        <details class="tools">
          <summary>{t.used(l.actions.length)}</summary>
          <ul class="well">
            {#each l.actions as a, j (j)}
              <li>
                <span class="dot {a.ok ? 'done' : 'wrong'}"></span>
                <span class="mono">{a.tool}</span>
                <span class="dim">{a.ok ? a.summary : `${t.failed}: ${a.summary}`}</span>
              </li>
            {/each}
          </ul>
        </details>
      {/if}
    </li>
  {/each}
  {#if live && store.hearing}
    <!-- What the user is saying, as it's written: the chat line replaces it. -->
    <li class="line you live" aria-live="polite">
      <div class="meta"><span class="who">{t.you}</span><span class="tag">{t.voice}</span></div>
      <p class="text">{store.hearing}</p>
    </li>
  {/if}
  {#if live && store.asking}
    <!-- The question itself is in the log already: the Rust side adds it before thinking. -->
    <li class="line yoru thinking" aria-live="polite">
      <span class="dots"><i></i><i></i><i></i></span>
    </li>
  {/if}
</ol>

{#if live && store.confirmations.length}
  <div class="cards">
    {#each store.confirmations as c (c.id)}<ConfirmCard card={c} />{/each}
  </div>
{/if}

<style>
  .log {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 18px;
  }
  .line {
    max-width: 88%;
  }
  .line.you {
    justify-self: end;
    text-align: right;
  }
  .meta {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: var(--fs-small);
    margin-bottom: 3px;
  }
  .you .meta {
    justify-content: flex-end;
  }
  .who {
    font-weight: 600;
    color: var(--haze);
  }
  .text {
    font-size: var(--fs-body);
    line-height: 1.55;
    white-space: pre-wrap;
    user-select: text;
  }
  .you .text {
    display: inline-block;
    padding: 8px 13px;
    border-radius: 16px 16px 4px 16px;
    background: var(--ink-3);
    text-align: left;
  }
  .live .text {
    opacity: 0.7;
  }
  .failed .text {
    color: var(--wrong);
  }
  .tools {
    margin-top: 8px;
    font-size: var(--fs-meta);
  }
  summary {
    color: var(--dusk);
    cursor: pointer;
    list-style: none;
    width: fit-content;
  }
  summary::before {
    content: "▸ ";
  }
  details[open] summary::before {
    content: "▾ ";
  }
  summary:hover {
    color: var(--haze);
  }
  .tools ul {
    list-style: none;
    margin: 8px 0 0;
    padding: 8px 12px;
    display: grid;
    gap: 6px;
  }
  .tools li {
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }
  .tools li .dim {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dots {
    display: inline-flex;
    gap: 5px;
    padding: 18px 0;
  }
  .dots i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--think);
    animation: dot 1.1s ease-in-out infinite;
  }
  .dots i:nth-child(2) {
    animation-delay: 160ms;
  }
  .dots i:nth-child(3) {
    animation-delay: 320ms;
  }
  .cards {
    display: grid;
    gap: 10px;
    margin-top: 20px;
  }
  @keyframes dot {
    50% {
      opacity: 0.25;
      transform: translateY(-2px);
    }
  }
</style>
