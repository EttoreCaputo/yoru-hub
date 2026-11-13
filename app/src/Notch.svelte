<!--
  Yoru in the notch (docs/notch.md). One black shape that grows out of the
  camera's notch, so it reads as part of the hardware: pure black, flush
  with the top of the screen, its top corners curving out into the menu
  bar, nothing drawn under the camera. Closed, a little Yoru sits on its
  left and what he's doing on its right. With the pointer on him, or while
  he hears or talks, it opens downwards: the last exchange as plain text,
  the cards to allow or deny, a field and a button that turns the microphone on and, clicked
  again, off. No
  cards, no greys: white text at a few strengths, and pills.

  The window is bigger than all this and lets clicks through but on what
  is drawn here: `notch_content` says where that is, and the Rust side
  says when the pointer is on it (`notch-pointer`).
-->
<script lang="ts">
  import { call, on } from "./lib/api";
  import { i18n } from "./lib/i18n/index.svelte";
  import { store } from "./lib/store.svelte";
  import type { VoiceState } from "./lib/types";
  import AgentList from "./lib/ui/AgentList.svelte";
  import YoruBoard from "./lib/YoruBoard.svelte";

  type Geometry = { notch: boolean; width: number; height: number; window: number };
  type NotchEvent =
    | { event: "state"; state: string }
    | { event: "level"; level: number }
    | { event: "react"; name: string }
    | { event: "fetch"; state: string; source: string; kind: string }
    | { event: "error"; message: string };

  store.start();
  i18n.start();
  const t = $derived(i18n.t.notch);
  const tc = $derived(i18n.t.card);

  const PANEL = 460;
  /** How long an answer stays open once Yoru is done, to be read. */
  const LINGER_MS = 6000;
  /** The pointer has to rest a moment to open it, and be away a moment to close it. */
  const ENTER_MS = 120;
  const LEAVE_MS = 350;

  let geometry = $state<Geometry>({ notch: false, width: 190, height: 32, window: 620 });
  let voice = $state<VoiceState>("idle");
  let level = $state(0);
  let reaction = $state<{ name: string; at: number } | null>(null);
  let fetching = $state<string | null>(null);
  let error = $state("");
  let pointer = $state(false);
  let lingerUntil = $state(0);
  let held = $state(false);
  let typed = $state("");
  let typing = $state(false);
  let answering = $state<string | null>(null);
  /** The microphone's permission: granted, denied, restricted, unknown. */
  let mic = $state("granted");
  let shape: HTMLElement | undefined = $state();

  const ear = $derived(geometry.height + 8);
  const closedWidth = $derived(geometry.width + 2 * ear);
  const busy = $derived(voice !== "idle" && voice !== "error");
  const open = $derived(pointer || busy || held || typing || !!store.asking || store.now < lingerUntil);
  const last = $derived.by(() => {
    const lastOf = (from: "you" | "yoru") => store.chat.map((l) => l.from).lastIndexOf(from);
    const yoru = lastOf("yoru");
    const you = lastOf("you");
    return { you: you >= 0 ? store.chat[you] : null, yoru: yoru > you ? store.chat[yoru] : null };
  });
  const card = $derived(store.confirmations[0] ?? null);
  /** What the left of the top row says while open. */
  const status = $derived(error ? t.states.error : fetching && busy ? fetching : t.states[voice] || "Yoru");

  /** Ask where it stands; macOS's own prompt comes up if it never did. */
  async function checkMic(ask: boolean) {
    try {
      mic = await call<string>("notch_microphone", { ask });
    } catch {
      // No helper (the browser, a broken install): talking will say why.
    }
  }
  async function allowMic() {
    if (mic === "unknown") await checkMic(true);
    else await call("open_microphone_privacy").catch(() => {});
  }
  void checkMic(false).then(() => {
    if (mic === "unknown") return checkMic(true);
  });
  // Settings may have changed it meanwhile: look again on opening.
  $effect(() => {
    if (pointer && mic !== "granted") void checkMic(false);
  });

  call<Geometry | null>("notch_geometry").then((g) => g && (geometry = g), () => {});
  on<Geometry>("notch-geometry", (g) => (geometry = g));
  let timer: ReturnType<typeof setTimeout> | undefined;
  on<boolean>("notch-pointer", (over) => {
    clearTimeout(timer);
    timer = setTimeout(() => (pointer = over), over ? ENTER_MS : LEAVE_MS);
  });
  on<NotchEvent>("notch", (e) => {
    if (e.event === "state") {
      const was = voice;
      voice = (e.state as VoiceState) ?? "idle";
      if (voice !== "error") error = "";
      if (voice === "idle" && was !== "idle") lingerUntil = Date.now() + LINGER_MS;
      if (voice !== "listening") level = 0;
      // The turn ended on its own (an error, or Yoru stopped): the button is off.
      if (held && (voice === "idle" || voice === "error")) held = false;
    } else if (e.event === "level") level = e.level;
    else if (e.event === "react") reaction = { name: e.name, at: Date.now() };
    else if (e.event === "fetch") fetching = e.state === "start" ? e.source || t.fetching : null;
    else if (e.event === "error") {
      error = e.message;
      voice = "error";
      void checkMic(false);
    }
  });

  // Say where the drawing is, as it opens and closes.
  $effect(() => {
    if (!shape) return;
    const report = () => {
      const r = shape!.getBoundingClientRect();
      void call("notch_content", { rect: { x: r.x, y: r.y, w: r.width, h: r.height } }).catch(() => {});
    };
    const watch = new ResizeObserver(report);
    watch.observe(shape);
    report();
    return () => watch.disconnect();
  });

  function talk(pressed: boolean) {
    if (pressed && mic !== "granted") return void allowMic();
    if (held === pressed) return;
    held = pressed;
    void call("notch_talk", { pressed }).catch(() => {});
  }

  async function send(e: SubmitEvent) {
    e.preventDefault();
    const text = typed.trim();
    if (!text || store.asking) return;
    typed = "";
    await store.ask(text);
    lingerUntil = Date.now() + LINGER_MS;
  }

  async function answer(allow: boolean) {
    if (!card || answering) return;
    answering = card.id;
    try {
      await store.confirm(card.id, allow);
    } finally {
      answering = null;
    }
  }

  /** A few bars for how loud the microphone is. */
  const bars = $derived([0.5, 0.85, 1, 0.7].map((k) => Math.max(0.2, Math.min(1, level * k * 1.5))));
</script>

<div
  class="shape"
  class:open
  bind:this={shape}
  style:--w="{open ? PANEL : closedWidth}px"
  style:--bar="{geometry.height}px"
  style:--ear="{ear}px"
  style:--notch="{geometry.width}px"
>
  <div class="top">
    <div class="side left">
      <span class="yoru"><YoruBoard board="live" size={geometry.height - 8} {voice} {reaction} /></span>
      {#if open}<span class="status" class:err={!!error}>{status}</span>{/if}
    </div>
    <div class="side right">
      {#if voice === "listening"}
        <span class="bars" aria-hidden="true">{#each bars as b, i (i)}<i style:--h={b}></i>{/each}</span>
      {:else if voice === "thinking" || store.asking}
        <span class="spinner" aria-hidden="true"></span>
      {:else if voice === "speaking"}
        <span class="wave" aria-hidden="true"><i></i><i></i><i></i></span>
      {:else if error}
        <span class="dot err"></span>
      {:else if mic !== "granted"}
        <svg class="mic-off" viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M4 4l16 16" /></svg>
      {:else if store.confirmations.length}
        <span class="count wait">{store.confirmations.length}</span>
      {:else if store.unread}
        <span class="count">{store.unread}</span>
      {:else if store.working}
        <span class="count agents" title={i18n.t.agents.title}>{store.working}</span>
      {/if}
    </div>
  </div>

  <div class="body">
    <div class="inner">
      {#if mic !== "granted"}
        <section class="ask">
          <p class="label"><span class="dot wait"></span>{mic === "unknown" ? t.micAsk : t.micDenied}</p>
          <div class="pills">
            <button class="pill light" onclick={allowMic}>{mic === "unknown" ? t.micAllow : t.micSettings}</button>
          </div>
        </section>
      {/if}
      {#if card}
        <section class="ask">
          <p class="label"><span class="dot wait"></span>{tc.yoruAsks}<span class="tool">{card.tool}</span></p>
          <p class="what">{card.text}</p>
          <div class="pills">
            <button class="pill light" disabled={!!answering} onclick={() => answer(true)}>{tc.allow}</button>
            <button class="pill" disabled={!!answering} onclick={() => answer(false)}>{tc.deny}</button>
            {#if store.confirmations.length > 1}<span class="more">+{store.confirmations.length - 1}</span>{/if}
          </div>
        </section>
      {:else if store.asking}
        <p class="you">{store.asking}</p>
      {:else if last.you || last.yoru}
        <section class="exchange">
          {#if last.you}<p class="you">{last.you.text}</p>{/if}
          {#if last.yoru}<p class="answer">{last.yoru.text}</p>{/if}
        </section>
      {:else}
        <p class="hint">{t.empty}</p>
      {/if}
      {#if error || store.askError}<p class="error">{error || store.askError}</p>{/if}
      {#if store.agents.length}
        <section class="at-work">
          <p class="label">{i18n.t.agents.title}</p>
          <AgentList limit={3} dark />
        </section>
      {/if}

      <div class="dock">
        <form class="field" onsubmit={send}>
          <input
            bind:value={typed}
            placeholder={store.asking ? i18n.t.composer.thinking : i18n.t.composer.placeholder}
            disabled={!!store.asking}
            onfocus={() => (typing = true)}
            onblur={() => (typing = false)}
            spellcheck="false"
          />
          <kbd>{t.key}</kbd>
        </form>
        <button
          class="round mic"
          class:on={held || voice === "listening"}
          title={held ? t.micStop : t.micStart}
          aria-label={held ? t.micStop : t.micStart}
          aria-pressed={held}
          onclick={() => talk(!held)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21" /></svg>
        </button>
        {#if busy}
          <button class="round" title={t.stop} aria-label={t.stop} onclick={() => call("notch_hush").catch(() => {})}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="2" class="solid" /></svg>
          </button>
        {/if}
        <button class="round" title={t.open} aria-label={t.open} onclick={() => call("show_app").catch(() => {})}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 4h6v6M20 4l-8 8M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4" /></svg>
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  :global(html.notch),
  :global(html.notch body) {
    background: transparent;
    overflow: hidden;
  }

  /* One black island: flush with the top, as wide as the notch and its ears. */
  .shape {
    --r: calc(var(--bar) * 0.42);
    --shoulder: 8px;
    --spring: cubic-bezier(0.32, 1.25, 0.5, 1);
    position: absolute;
    top: 0;
    left: 50%;
    width: var(--w);
    transform: translateX(-50%);
    background: #000;
    color: #fff;
    border-radius: 0 0 var(--r) var(--r);
    font: 400 13px/1.45 var(--sans);
    transition:
      width 0.42s var(--spring),
      border-radius 0.42s var(--spring),
      box-shadow 0.3s ease;
  }
  .shape.open {
    --r: 30px;
    box-shadow: 0 18px 50px rgba(0, 0, 0, 0.55);
  }
  /* The top corners curve out into the menu bar, as the notch's own do. */
  .shape::before,
  .shape::after {
    content: "";
    position: absolute;
    top: 0;
    width: var(--shoulder);
    height: var(--shoulder);
    background: radial-gradient(circle at 0 100%, transparent calc(var(--shoulder) - 0.5px), #000 var(--shoulder));
  }
  .shape::before {
    left: calc(-1 * var(--shoulder));
  }
  .shape::after {
    right: calc(-1 * var(--shoulder));
    transform: scaleX(-1);
  }

  /* The row beside the camera: nothing in its middle. */
  .top {
    display: flex;
    justify-content: space-between;
    height: var(--bar);
    padding: 0 calc((var(--ear) - var(--bar) + 8px) / 2);
  }
  .side {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    max-width: calc((var(--w) - var(--notch)) / 2 - 12px);
  }
  .yoru {
    display: grid;
    flex: none;
  }
  .status {
    font-weight: 600;
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    animation: fade 0.3s 0.1s ease both;
  }
  .status.err {
    color: #ff6b6b;
  }

  .bars,
  .wave {
    display: flex;
    gap: 2.5px;
    align-items: center;
    height: 14px;
  }
  .bars i {
    width: 3px;
    height: calc(var(--h) * 14px);
    border-radius: 2px;
    background: #40e080;
    transition: height 0.08s linear;
  }
  .wave i {
    width: 3px;
    height: 14px;
    border-radius: 2px;
    background: #40e080;
    animation: wave 0.9s ease-in-out infinite;
  }
  .wave i:nth-child(2) {
    animation-delay: 0.15s;
  }
  .wave i:nth-child(3) {
    animation-delay: 0.3s;
  }
  .spinner {
    width: 13px;
    height: 13px;
    border-radius: 50%;
    border: 2px solid rgba(74, 163, 255, 0.25);
    border-top-color: #4aa3ff;
    animation: spin 0.8s linear infinite;
  }
  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
  }
  .dot.err {
    background: #ff4d4d;
  }
  .dot.wait {
    background: #ff9f43;
  }
  .mic-off {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: #ff9f43;
    stroke-width: 2;
    stroke-linecap: round;
  }
  .count {
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.14);
    font: 600 10.5px/18px var(--sans);
    text-align: center;
  }
  .count.wait {
    background: #ff9f43;
    color: #000;
  }
  /* Coding agents working: the firmware's thinking tint. */
  .count.agents {
    background: rgba(74, 163, 255, 0.85);
    color: #000;
  }
  .at-work {
    margin-top: 10px;
  }
  .at-work .label {
    color: rgba(255, 255, 255, 0.55);
  }

  /* What opens below: its height eases with the width. */
  .body {
    display: grid;
    grid-template-rows: 0fr;
    transition: grid-template-rows 0.42s var(--spring);
  }
  .open .body {
    grid-template-rows: 1fr;
  }
  .inner {
    overflow: hidden;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 0 20px;
    opacity: 0;
    filter: blur(6px);
    transform: scale(0.97);
    transform-origin: top center;
    transition:
      opacity 0.2s ease,
      filter 0.2s ease,
      transform 0.3s ease,
      padding 0.42s var(--spring);
  }
  .open .inner {
    padding: 10px 20px 18px;
    opacity: 1;
    filter: none;
    transform: none;
    transition-delay: 0.08s;
  }

  p {
    margin: 0;
  }
  .exchange {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 260px;
    overflow-y: auto;
    scrollbar-width: none;
  }
  .you {
    color: rgba(255, 255, 255, 0.5);
    font-size: 12.5px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .answer {
    font-size: 15px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .hint {
    color: rgba(255, 255, 255, 0.45);
  }
  .error {
    color: #ff6b6b;
    font-size: 12px;
  }

  .ask {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    font-weight: 600;
    color: #ffb46b;
  }
  .tool {
    font: 400 11.5px var(--mono);
    color: rgba(255, 255, 255, 0.4);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .what {
    font-size: 14.5px;
  }
  .pills {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .more {
    color: rgba(255, 255, 255, 0.4);
    font-size: 12px;
  }

  button {
    font: inherit;
    color: inherit;
    border: 0;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
  }
  .pill {
    height: 30px;
    padding: 0 16px;
    border-radius: 15px;
    background: rgba(255, 255, 255, 0.12);
    font-weight: 600;
    transition: background 0.15s ease;
  }
  .pill:hover {
    background: rgba(255, 255, 255, 0.2);
  }
  .pill.light {
    background: #fff;
    color: #000;
  }
  .pill.light:hover {
    background: rgba(255, 255, 255, 0.85);
  }

  .dock {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .field {
    flex: 1;
    display: flex;
    align-items: center;
    height: 36px;
    padding: 0 6px 0 14px;
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.08);
    transition: background 0.15s ease;
  }
  .field:focus-within {
    background: rgba(255, 255, 255, 0.13);
  }
  .field input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: 0;
    color: #fff;
    font: inherit;
  }
  .field input::placeholder {
    color: rgba(255, 255, 255, 0.38);
  }
  kbd {
    padding: 2px 7px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.45);
    font: 500 10.5px var(--mono);
    white-space: nowrap;
  }
  .field:focus-within kbd {
    display: none;
  }
  .round {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.08);
    transition:
      background 0.15s ease,
      transform 0.15s ease;
  }
  .round:hover {
    background: rgba(255, 255, 255, 0.16);
  }
  .round svg {
    width: 17px;
    height: 17px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .round .solid {
    fill: currentColor;
    stroke: none;
  }
  .mic {
    user-select: none;
  }
  .mic.on {
    background: #40e080;
    color: #000;
    transform: scale(1.08);
  }

  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }
  @keyframes wave {
    50% {
      transform: scaleY(0.35);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .shape,
    .body,
    .inner {
      transition-duration: 0.01s;
    }
  }
</style>
