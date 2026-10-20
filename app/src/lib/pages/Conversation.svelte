<!--
  The conversation with Yoru, typed or spoken: its tools folded under each
  answer, and the cards it's waiting on right where it stopped.
-->
<script lang="ts">
  import { call } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ChatLine } from "../types";
  import ApprovalCard from "../ui/ApprovalCard.svelte";
  import Composer from "../ui/Composer.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import { ago } from "../ui/when";

  const t = $derived(i18n.t.conversation);
  let end: HTMLElement | undefined = $state();

  // Down to the newest line whenever one comes, or a card, or the wait.
  $effect(() => {
    void store.chat.length;
    void store.yoruApprovals.length;
    void store.asking;
    queueMicrotask(() => end?.scrollIntoView({ block: "end", behavior: "smooth" }));
  });

  const failed = (l: ChatLine) => l.from === "yoru" && l.text.startsWith("(error:");
</script>

<div class="page convo">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    {#if store.chat.length}
      <div class="row">
        <ConfirmButton label={t.forget} question={t.forgetConfirm} sure={t.forgetSure} onconfirm={() => call("forget_conversation")} />
      </div>
    {/if}
  </div>

  {#if !store.chat.length && !store.asking}
    <div class="empty">
      <b>{t.empty}</b>
      <p>{t.emptyHint}</p>
      <div class="row suggestions">
        {#each t.suggestions as s (s)}
          <button class="btn" disabled={!store.info?.ready} onclick={() => store.ask(s)}>{s}</button>
        {/each}
      </div>
    </div>
  {/if}

  <ol class="log">
    {#each store.chat as l, i (i)}
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
            <ul>
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
    {#if store.asking}
      <!-- The question itself is in the log already: the Rust side adds it before thinking. -->
      <li class="line yoru thinking" aria-live="polite">
        <span class="dots"><i></i><i></i><i></i></span>
      </li>
    {/if}
  </ol>

  {#if store.yoruApprovals.length}
    <div class="cards">
      {#each store.yoruApprovals as a (a.id)}<ApprovalCard approval={a} />{/each}
    </div>
  {/if}

  <div class="dock" bind:this={end}>
    <Composer autofocus />
  </div>
</div>

<style>
  .convo {
    display: flex;
    flex-direction: column;
    min-height: 100%;
    max-width: 760px;
  }
  .suggestions {
    justify-content: center;
    margin-top: 18px;
  }
  .log {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 22px;
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
    font-size: 12px;
    margin-bottom: 4px;
  }
  .you .meta {
    justify-content: flex-end;
  }
  .who {
    font-weight: 600;
    color: var(--haze);
  }
  .text {
    font-size: 15px;
    line-height: 1.55;
    white-space: pre-wrap;
    user-select: text;
  }
  .you .text {
    display: inline-block;
    padding: 9px 14px;
    border-radius: 18px 18px 4px 18px;
    background: var(--ink-3);
    text-align: left;
  }
  .yoru .text {
    font-size: 15.5px;
  }
  .failed .text {
    color: var(--wrong);
  }
  .tools {
    margin-top: 8px;
    font-size: 12.5px;
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
    border-radius: 10px;
    background: var(--ink);
    border: 1px solid var(--rim);
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
    padding: 10px 0;
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
  .dock {
    position: sticky;
    bottom: 0;
    margin-top: auto;
    padding: 24px 0 4px;
    background: linear-gradient(transparent, var(--night) 40%);
  }
  @keyframes dot {
    50% {
      opacity: 0.25;
      transform: translateY(-2px);
    }
  }
</style>
