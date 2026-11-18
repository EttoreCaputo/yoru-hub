<!--
  Home: Yoru as a mascot, the conversation with him, and the field to type
  or talk at the bottom; on the right, the activity panel (agents, tasks,
  routines), which folds away.

  The mascot is his own, not the board's mirror: awake whether or not a
  board is on the desk, he thinks while an answer is on its way, listens
  and talks with the Mac's voice (and the board's), and reacts to what's
  said in the chat (mood.ts) and to the reactions Yoru plays.
-->
<script lang="ts">
  import { call } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { reactionFor } from "../mood";
  import { store } from "../store.svelte";
  import type { VoiceState } from "../types";
  import ActivityPanel from "../ui/ActivityPanel.svelte";
  import ChatLog from "../ui/ChatLog.svelte";
  import Composer from "../ui/Composer.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import YoruBoard from "../YoruBoard.svelte";

  const t = $derived(i18n.t.home);
  const PANEL_KEY = "yoru.activity";

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

  let panelOpen = $state(read());
  function read() {
    try {
      return localStorage.getItem(PANEL_KEY) !== "closed";
    } catch {
      return true;
    }
  }
  $effect(() => {
    try {
      localStorage.setItem(PANEL_KEY, panelOpen ? "open" : "closed");
    } catch {
      // The panel just opens next time.
    }
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
</script>

<div class="home">
  <section class="talk">
    <div class="mascot" class:small={!empty}>
      <div class="yoru">
        <YoruBoard board="live" size={empty ? 210 : 116} {voice} gaze={store.gaze} {badge} {reaction} onpoke={() => play("happy")} />
      </div>
      <div class="words">
        <p class="eyebrow">{greeting}</p>
        <h1 class="said">{line}</h1>
        {#if !ready}<button class="btn primary" onclick={() => store.go("settings")}>{t.noKeyAction}</button>{/if}
      </div>
      {#if store.chat.length}
        <div class="forget">
          <ConfirmButton label={t.forget} question={t.forgetConfirm} sure={t.forgetSure} onconfirm={() => call("forget_conversation")} />
        </div>
      {/if}
    </div>

    <div class="log" bind:this={log}>
      <div class="column">
        {#if empty && ready}
          <div class="suggestions">
            {#each t.suggestions as s (s)}
              <button class="btn" onclick={() => store.ask(s)}>{s}</button>
            {/each}
          </div>
        {/if}
        <ChatLog />
      </div>
    </div>

    <div class="dock">
      <div class="column"><Composer mic big autofocus /></div>
    </div>
  </section>

  <ActivityPanel bind:open={panelOpen} />
</div>

<style>
  .home {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    height: 100%;
  }
  .talk {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .column {
    max-width: 760px;
    margin: 0 auto;
    padding: 0 36px;
  }
  /* Yoru on his own: the sphere without the board's black disc around it. */
  .mascot {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 46px 36px 10px;
    text-align: center;
    transition: padding 300ms;
  }
  .mascot.small {
    flex-direction: row;
    justify-content: center;
    gap: 16px;
    padding-top: 40px;
    text-align: left;
  }
  .yoru :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .eyebrow {
    color: var(--haze);
    font-size: 13.5px;
  }
  .said {
    margin: 6px 0 0;
    font: 560 30px/1.15 var(--display);
    letter-spacing: -0.015em;
    max-width: 560px;
    text-wrap: balance;
  }
  .small .said {
    font-size: 21px;
  }
  .words .btn {
    margin-top: 16px;
  }
  .forget {
    position: absolute;
    right: 24px;
    bottom: 6px;
  }
  .log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 18px 0 8px;
    mask-image: linear-gradient(transparent, #000 24px);
  }
  .suggestions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin: 6px 0 20px;
  }
  .dock {
    padding: 8px 0 22px;
  }
</style>
