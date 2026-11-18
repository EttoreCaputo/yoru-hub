<!--
  Yoru at the bottom right of every page but the Home and a conversation's
  tab: the mascot, awake (his eyes follow the pointer, he thinks while an
  answer is on its way, a dot when he waits for a yes). A click opens his
  chat over the page, on the conversation he's in: to ask something without
  leaving the page, or to correct what he knows from Memory. It can grow
  into a tab of its own, or start a new conversation.
-->
<script lang="ts">
  import { call } from "./api";
  import { bubble } from "./bubble.svelte";
  import { i18n } from "./i18n/index.svelte";
  import { reactionFor } from "./mood";
  import { store } from "./store.svelte";
  import { tabs } from "./tabs.svelte";
  import type { VoiceState } from "./types";
  import ChatLog from "./ui/ChatLog.svelte";
  import Composer from "./ui/Composer.svelte";
  import Icon from "./ui/Icon.svelte";
  import YoruBoard from "./YoruBoard.svelte";

  const t = $derived(i18n.t.bubble);
  const voice = $derived<VoiceState>(store.asking ? "thinking" : store.talking ? "listening" : store.macVoice !== "idle" ? store.macVoice : store.voice);
  const badge = $derived(store.confirmations.length ? "wait" : null);
  const title = $derived(store.chatTitle?.text || t.title);

  let reaction = $state<{ name: string; at: number } | null>(null);
  let seen = store.chat.length;
  $effect(() => {
    const all = store.chat;
    if (all.length < seen) seen = 0;
    const fresh = all.slice(seen);
    seen = all.length;
    const name = fresh.length === 1 ? reactionFor(fresh[0]) : null;
    if (name) reaction = { name, at: Date.now() };
  });
  $effect(() => {
    if (store.macReaction) reaction = store.macReaction;
  });

  // Down to the newest line whenever one comes.
  let log: HTMLElement | undefined = $state();
  $effect(() => {
    void store.chat.length;
    void store.asking;
    void bubble.open;
    queueMicrotask(() => log?.scrollTo({ top: log.scrollHeight }));
  });

  function grow() {
    tabs.open({ kind: "chat", id: store.currentChat });
    bubble.hide();
  }
  async function fresh() {
    store.currentChat = await call<string>("new_conversation");
  }
  function keys(e: KeyboardEvent) {
    if (e.key === "Escape" && bubble.open) bubble.hide();
  }
</script>

<svelte:window onkeydown={keys} />

<div class="spot">
  {#if bubble.open}
    <div class="chat" role="dialog" aria-label={t.title}>
      <header>
        <b class:faint={!store.chatTitle?.text}>{title}</b>
        <button class="icon-btn small" title={t.new} aria-label={t.new} onclick={fresh}><Icon name="plus" size={16} /></button>
        <button class="icon-btn small" title={t.grow} aria-label={t.grow} onclick={grow}><Icon name="maximize-2" size={15} /></button>
        <button class="icon-btn small" title={t.close} aria-label={t.close} onclick={() => bubble.hide()}><Icon name="x" size={16} /></button>
      </header>
      <div class="log" bind:this={log}>
        {#if !store.chat.length && !store.asking && !store.hearing}
          <p class="hello faint">{t.hello}</p>
        {/if}
        <ChatLog />
      </div>
      <div class="field">
        <Composer mic autofocus bind:text={bubble.draft} />
      </div>
    </div>
  {/if}

  <div class="yoru" class:open={bubble.open} title={bubble.open ? t.close : t.open}>
    <YoruBoard board="live" size={100} {voice} {badge} {reaction} follow onpoke={() => (bubble.open ? bubble.hide() : bubble.show())} />
  </div>
</div>

<style>
  /* Over the page's card, at its bottom right; only Yoru and his chat take clicks. */
  .spot {
    grid-column: 2;
    grid-row: 2;
    align-self: end;
    justify-self: end;
    z-index: 6;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0;
    margin: 0 10px 6px 0;
    pointer-events: none;
  }
  .spot > * {
    pointer-events: auto;
  }
  /* The sphere alone, without the board's black disc. */
  .yoru :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .yoru {
    transition: transform 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .yoru:hover {
    transform: translateY(-2px);
  }
  .chat {
    display: flex;
    flex-direction: column;
    width: min(400px, calc(100vw - 340px));
    height: min(540px, calc(100vh - 250px));
    border: 1px solid var(--rim-2);
    border-radius: var(--r-lg);
    background: var(--night);
    box-shadow: var(--shadow);
    overflow: hidden;
    transform-origin: bottom right;
    animation: pop 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.97);
    }
  }
  header {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 48px;
    padding: 0 8px 0 16px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--rim);
  }
  header b {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 600 var(--fs-body)/1.3 var(--sans);
  }
  .log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px 16px 8px;
  }
  .hello {
    margin: 4px 0 0;
    font-size: var(--fs-meta);
    text-align: center;
  }
  .field {
    padding: 8px 12px 12px;
  }
</style>
