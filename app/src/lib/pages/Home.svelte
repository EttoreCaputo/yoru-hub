<!--
  Home: Yoru as a mascot, the conversation with him, and the field to type
  or talk at the bottom.

  The mascot is his own, not the board's mirror: awake whether or not a
  board is on the desk, he thinks while an answer is on its way, listens
  and talks with the Mac's voice (and the board's), and reacts to what's
  said in the chat (mood.ts) and to the reactions Yoru plays. With no
  conversation he's in the middle with a greeting; in one, he's small, left
  of the newest line. His eyes follow the pointer.
-->
<script lang="ts">
  import { call } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { reactionFor } from "../mood";
  import { store } from "../store.svelte";
  import type { VoiceState } from "../types";
  import ChatLog from "../ui/ChatLog.svelte";
  import Composer from "../ui/Composer.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import YoruBoard from "../YoruBoard.svelte";

  const t = $derived(i18n.t.home);

  const greeting = $derived.by(() => {
    const h = new Date(store.now).getHours();
    return h < 5 ? t.greeting.night : h < 12 ? t.greeting.morning : h < 18 ? t.greeting.afternoon : h < 23 ? t.greeting.evening : t.greeting.night;
  });
  const ready = $derived(store.info?.ready ?? true);
  const line = $derived(!ready ? t.noKey : store.unread ? t.unreadLine(store.unread) : t.quietLine);
  const empty = $derived(!store.chat.length && !store.asking && !store.hearing);

  // What he's doing: thinking over a typed question, else the Mac's voice, else the board's.
  const voice = $derived<VoiceState>(
    store.asking ? "thinking" : store.talking ? "listening" : store.macVoice !== "idle" ? store.macVoice : store.voice,
  );
  // The conversation's title: Yoru names it as it goes on; a click renames it.
  const title = $derived(store.chatTitle?.text ?? "");
  let renaming = $state(false);
  let renamed = false;
  function rename(text: string) {
    if (renamed || !renaming) return;
    renamed = true;
    renaming = false;
    if (text.trim() !== title) void call("set_chat_title", { text: text.trim() });
    queueMicrotask(() => (renamed = false));
  }
  function renameKey(e: KeyboardEvent) {
    if (e.key === "Enter") rename((e.currentTarget as HTMLInputElement).value);
    else if (e.key === "Escape") renaming = false;
  }

  const badge = $derived(store.confirmations.length ? "wait" : null);

  // A reaction for each new line of the chat, and for each one Yoru plays.
  let reaction = $state<{ name: string; at: number } | null>(null);
  const play = (name: string) => (reaction = { name, at: Date.now() });
  let seen = store.chat.length;
  $effect(() => {
    const lines = store.chat;
    if (lines.length < seen) seen = 0;
    const fresh = lines.slice(seen);
    seen = lines.length;
    // Only the newest one: a reload of the log isn't a conversation.
    const last = fresh.length === 1 ? fresh[0] : null;
    const name = last && reactionFor(last);
    if (name) play(name);
  });
  $effect(() => {
    if (store.macReaction) reaction = store.macReaction;
  });
  // Hello, once the Home opens.
  $effect(() => {
    const hello = setTimeout(() => play("wave"), 500);
    return () => clearTimeout(hello);
  });

  // Down to the newest line whenever one comes, or a card, or the wait.
  let log: HTMLElement | undefined = $state();
  $effect(() => {
    void store.chat.length;
    void store.confirmations.length;
    void store.asking;
    void store.hearing;
    queueMicrotask(() => log?.scrollTo({ top: log.scrollHeight, behavior: "smooth" }));
  });

  // In the conversation Yoru sits left of the newest line (his answer, your
  // question, his thinking dots), and moves down with it.
  let column: HTMLElement | undefined = $state();
  let avatarTop = $state(0);
  /** The board's size: the sphere in it is about 40% of that, about 30px. */
  const AVATAR = 72;
  const box = AVATAR * 1.15;
  function place() {
    const lines = column?.querySelectorAll<HTMLElement>(":scope .log > li");
    const last = lines?.[lines.length - 1];
    if (last) avatarTop = last.offsetTop + Math.min(last.offsetHeight, 48) / 2 - box / 2;
  }
  $effect(() => {
    void store.chat.length;
    void store.asking;
    void store.hearing;
    if (!column) return;
    queueMicrotask(place);
    const watch = new ResizeObserver(place);
    watch.observe(column);
    return () => watch.disconnect();
  });
</script>

<section class="talk">
  {#if empty}
    <div class="mascot">
      <div class="yoru">
        <YoruBoard board="live" size={150} {voice} {badge} {reaction} follow onpoke={() => play("happy")} />
      </div>
      <p class="eyebrow">{greeting}</p>
      <h1 class="said">{line}</h1>
      {#if !ready}<button class="btn primary" onclick={() => store.go("settings")}>{t.noKeyAction}</button>{/if}
    </div>
  {:else}
    <header class="head">
      {#if renaming}
        <input
          class="rename"
          type="text"
          value={store.chatTitle?.text ?? ""}
          placeholder={t.untitled}
          title={t.renameHint}
          aria-label={t.rename}
          {@attach (el: HTMLInputElement) => (el.focus(), el.select())}
          onkeydown={renameKey}
          onblur={(e) => rename(e.currentTarget.value)}
        />
      {:else}
        <button class="title" onclick={() => (renaming = true)} title={t.rename}>
          <span class:faint={!title}>{title || t.untitled}</span>
          <span class="pen"><Icon name="pencil" size={13} /></span>
        </button>
      {/if}
      <div class="tools">
        <ConfirmButton icon="rotate-ccw" label={t.forget} question={t.forgetConfirm} sure={t.forgetSure} onconfirm={() => call("forget_conversation")} />
      </div>
    </header>
  {/if}

  <div class="log" bind:this={log}>
    <div class="column" class:chat={!empty} bind:this={column}>
      {#if empty && ready}
        <div class="suggestions">
          {#each t.suggestions as s (s)}
            <button class="btn" onclick={() => store.ask(s)}>{s}</button>
          {/each}
        </div>
      {/if}
      {#if !empty}
        <div class="avatar yoru" style:top="{avatarTop}px">
          <YoruBoard board="live" size={AVATAR} {voice} {badge} {reaction} follow onpoke={() => play("happy")} />
        </div>
      {/if}
      <ChatLog />
    </div>
  </div>

  <div class="dock">
    <div class="column" class:chat={!empty}><Composer mic big autofocus /></div>
  </div>
</section>

<style>
  .talk {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .column {
    position: relative;
    max-width: 760px;
    margin: 0 auto;
    padding: 0 36px;
  }
  /* Room on the left for Yoru beside the newest line. */
  .column.chat {
    max-width: 800px;
    padding-left: 76px;
  }
  /* Yoru on his own: the sphere without the board's black disc around it. */
  .mascot {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 48px 36px 8px;
    text-align: center;
  }
  .yoru :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .eyebrow {
    margin-top: 4px;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .said {
    margin: 4px 0 0;
    max-width: 480px;
    font: 600 var(--fs-head)/1.35 var(--display);
    letter-spacing: -0.01em;
    text-wrap: balance;
  }
  .mascot .btn {
    margin-top: 14px;
  }
  .avatar {
    position: absolute;
    left: 2px;
    transition: top 320ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  /* The chat's head: its title on the left, its tools on the right. */
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 52px;
    padding: 0 12px 0 20px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--rim);
  }
  .title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 100%;
    margin-left: -8px;
    padding: 4px 8px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--pearl);
    font: 600 var(--fs-body)/1.3 var(--sans);
    cursor: text;
    transition: background 140ms;
  }
  .title span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title:hover {
    background: var(--ink-2);
  }
  .pen {
    color: var(--dusk);
    opacity: 0;
    transition: opacity 140ms;
  }
  .title:hover .pen,
  .title:focus-visible .pen {
    opacity: 1;
  }
  .rename {
    width: min(420px, 100%);
    min-height: 30px;
    margin-left: -8px;
    padding: 4px 8px;
    font-weight: 600;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    flex-shrink: 0;
  }
  .log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 0 8px;
    mask-image: linear-gradient(transparent, #000 24px);
  }
  .suggestions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin: 0 0 20px;
  }
  .dock {
    padding: 8px 0 20px;
  }
</style>
