<!--
  A conversation, in a tab of its own: its title (Yoru names it, a click
  renames it), the lines, and the field to type or talk at the bottom.

  The one Yoru is in is live: the mascot thinks while an answer is on its
  way, listens and talks with the Mac's voice (and the board's), and reacts
  to what's said (mood.ts) and to the reactions Yoru plays; he sits small,
  left of the newest line, and his eyes follow the pointer. With nothing
  said yet he's in the middle with a greeting. An earlier conversation shows
  as it was; writing in it goes on with it (it becomes the one Yoru is in).
-->
<script lang="ts">
  import { call } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { reactionFor } from "../mood";
  import { store } from "../store.svelte";
  import type { SavedChat, VoiceState } from "../types";
  import ChatLog from "../ui/ChatLog.svelte";
  import Composer from "../ui/Composer.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import YoruBoard from "../YoruBoard.svelte";

  let { id }: { id: string } = $props();
  const t = $derived(i18n.t.home);

  const live = $derived(id === store.currentChat);
  // An earlier one, read when its tab opens and whenever the list changes.
  let saved = $state<SavedChat | null>(null);
  $effect(() => {
    void store.conversations;
    if (live) return;
    const want = id;
    call<SavedChat | null>("conversation", { id: want }).then((c) => want === id && (saved = c), () => {});
  });
  const lines = $derived(live ? store.chat : (saved?.lines ?? []));

  const greeting = $derived.by(() => {
    const h = new Date(store.now).getHours();
    return h < 5 ? t.greeting.night : h < 12 ? t.greeting.morning : h < 18 ? t.greeting.afternoon : h < 23 ? t.greeting.evening : t.greeting.night;
  });
  const ready = $derived(store.info?.ready ?? true);
  const line = $derived(!ready ? t.noKey : store.unread ? t.unreadLine(store.unread) : t.quietLine);
  const empty = $derived(live && !store.chat.length && !store.asking && !store.hearing);

  // What he's doing: thinking over a typed question, else the Mac's voice, else the board's.
  const voice = $derived<VoiceState>(
    !live ? "idle" : store.asking ? "thinking" : store.talking ? "listening" : store.macVoice !== "idle" ? store.macVoice : store.voice,
  );
  const title = $derived(live ? (store.chatTitle?.text ?? "") : (saved?.title.text ?? ""));
  let renaming = $state(false);
  let renamed = false;
  function rename(text: string) {
    if (renamed || !renaming) return;
    renamed = true;
    renaming = false;
    if (text.trim() !== title) void call("rename_conversation", { id, text: text.trim() });
    queueMicrotask(() => (renamed = false));
  }
  function renameKey(e: KeyboardEvent) {
    if (e.key === "Enter") rename((e.currentTarget as HTMLInputElement).value);
    else if (e.key === "Escape") renaming = false;
  }

  const badge = $derived(live && store.confirmations.length ? "wait" : null);

  // A reaction for each new line of the chat, and for each one Yoru plays.
  let reaction = $state<{ name: string; at: number } | null>(null);
  const play = (name: string) => (reaction = { name, at: Date.now() });
  let seen = store.chat.length;
  $effect(() => {
    const all = store.chat;
    if (all.length < seen) seen = 0;
    const fresh = all.slice(seen);
    seen = all.length;
    // Only the newest one, in this conversation: a reload of the log isn't one.
    const last = live && fresh.length === 1 ? fresh[0] : null;
    const name = last && reactionFor(last);
    if (name) play(name);
  });
  $effect(() => {
    if (live && store.macReaction) reaction = store.macReaction;
  });
  // Hello, once the tab opens.
  $effect(() => {
    const hello = setTimeout(() => play("wave"), 500);
    return () => clearTimeout(hello);
  });

  // Down to the newest line whenever one comes, or a card, or the wait.
  let log: HTMLElement | undefined = $state();
  $effect(() => {
    void lines.length;
    void store.confirmations.length;
    void store.asking;
    void store.hearing;
    queueMicrotask(() => log?.scrollTo({ top: log.scrollHeight, behavior: "smooth" }));
  });

  // Yoru sits left of the newest line (his answer, your question, his
  // thinking dots), and moves down with it.
  let column: HTMLElement | undefined = $state();
  let avatarTop = $state(0);
  /** The board's size: the sphere in it is about 40% of that, about 30px. */
  const AVATAR = 72;
  const box = AVATAR * 1.15;
  function place() {
    const items = column?.querySelectorAll<HTMLElement>(":scope .log > li");
    const last = items?.[items.length - 1];
    if (last) avatarTop = last.offsetTop + Math.min(last.offsetHeight, 48) / 2 - box / 2;
  }
  $effect(() => {
    void lines.length;
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
          value={title}
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
        <ConfirmButton icon="trash-2" label={t.forget} question={t.forgetConfirm} sure={t.forgetSure} onconfirm={() => call("delete_conversation", { id })} />
      </div>
    </header>
  {/if}

  <div class="log" bind:this={log}>
    <div class="column" class:chat={!empty} bind:this={column}>
      {#if empty && ready}
        <div class="suggestions">
          {#each t.suggestions as s (s)}
            <button class="btn" onclick={() => store.ask(s, false, id)}>{s}</button>
          {/each}
        </div>
      {/if}
      {#if !empty}
        <div class="avatar yoru" style:top="{avatarTop}px">
          <YoruBoard board="live" size={AVATAR} {voice} {badge} {reaction} follow onpoke={() => play("happy")} />
        </div>
      {/if}
      <ChatLog {lines} {live} />
    </div>
  </div>

  <div class="dock">
    <div class="column" class:chat={!empty}>
      {#if !live}<p class="faint reading">{t.reading}</p>{/if}
      <Composer mic autofocus conversation={id} />
    </div>
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
  .yoru :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .mascot {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 48px 36px 8px;
    text-align: center;
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
    background: var(--hover);
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
  .reading {
    margin: 0 0 8px;
    font-size: var(--fs-meta);
    text-align: center;
  }
  .dock {
    padding: 8px 0 20px;
  }
</style>
