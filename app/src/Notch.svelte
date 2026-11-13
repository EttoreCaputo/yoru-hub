<!--
  Yoru in the notch (docs/notch.md). Closed, he's a black shape around the
  camera: a little Yoru on its left, what he's doing on its right. With the
  pointer on him, or while he hears or talks, he opens into a panel: the
  last exchange, the cards waiting for an answer, a field to type in and a
  button to hold and talk.

  The window is bigger than all this and lets clicks through but on what
  is drawn here: `notch_content` says where that is, and the Rust side
  says when the pointer is on it (`notch-pointer`).
-->
<script lang="ts">
  import { call, on } from "./lib/api";
  import { i18n } from "./lib/i18n/index.svelte";
  import { store } from "./lib/store.svelte";
  import Composer from "./lib/ui/Composer.svelte";
  import ConfirmCard from "./lib/ui/ConfirmCard.svelte";
  import type { VoiceState } from "./lib/types";
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

  const PANEL = 440;
  /** How long an answer stays open once Yoru is done, to be read. */
  const LINGER_MS = 6000;
  /** How long the pointer may be away before it closes. */
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
  let shape: HTMLElement | undefined = $state();

  const ear = $derived(geometry.height + 6);
  const closedWidth = $derived(geometry.width + 2 * ear);
  const busy = $derived(voice !== "idle" && voice !== "error");
  const open = $derived(pointer || busy || held || store.now < lingerUntil);
  const last = $derived.by(() => {
    const lastOf = (from: "you" | "yoru") => store.chat.map((l) => l.from).lastIndexOf(from);
    const yoru = lastOf("yoru");
    const you = lastOf("you");
    return { you: you >= 0 ? store.chat[you] : null, yoru: yoru > you ? store.chat[yoru] : null };
  });

  call<Geometry | null>("notch_geometry").then((g) => g && (geometry = g), () => {});
  on<Geometry>("notch-geometry", (g) => (geometry = g));
  let leaving: ReturnType<typeof setTimeout> | undefined;
  on<boolean>("notch-pointer", (over) => {
    clearTimeout(leaving);
    if (over) pointer = true;
    else leaving = setTimeout(() => (pointer = false), LEAVE_MS);
  });
  on<NotchEvent>("notch", (e) => {
    if (e.event === "state") {
      const was = voice;
      voice = (e.state as VoiceState) ?? "idle";
      if (voice !== "error") error = "";
      if (voice === "idle" && was !== "idle") lingerUntil = Date.now() + LINGER_MS;
      if (voice !== "listening") level = 0;
    } else if (e.event === "level") level = e.level;
    else if (e.event === "react") reaction = { name: e.name, at: Date.now() };
    else if (e.event === "fetch") fetching = e.state === "start" ? e.source || t.fetching : null;
    else if (e.event === "error") {
      error = e.message;
      voice = "error";
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
    if (held === pressed) return;
    held = pressed;
    void call("notch_talk", { pressed }).catch(() => {});
  }

  /** A few bars for how loud the microphone is. */
  const bars = $derived([0.55, 1, 0.75].map((k) => Math.max(0.18, Math.min(1, level * k * 1.4))));
</script>

<div
  class="shape"
  class:open
  class:soft={!geometry.notch}
  bind:this={shape}
  style:--w="{open ? PANEL : closedWidth}px"
  style:--bar="{geometry.height}px"
  style:--notch="{geometry.width}px"
  style:--ear="{ear}px"
>
  <div class="top">
    <div class="ear left">
      {#if !open}<YoruBoard board="live" size={geometry.height - 6} {voice} {reaction} />{/if}
    </div>
    <div class="ear right">
      {#if voice === "listening"}
        <span class="bars" aria-hidden="true">{#each bars as b, i (i)}<i style:--h={b}></i>{/each}</span>
      {:else if busy || voice === "error"}
        <span class="dot {voice}"></span>
      {:else if store.confirmations.length}
        <span class="count wait">{store.confirmations.length}</span>
      {:else if store.unread}
        <span class="count">{store.unread}</span>
      {/if}
    </div>
  </div>

  {#if open}
    <div class="panel">
      <div class="who">
        <YoruBoard board="live" size={84} {voice} {reaction} />
        <div class="state">
          <b>{t.states[voice] || (fetching ?? "Yoru")}</b>
          {#if error}<span class="error-text">{error}</span>
          {:else if fetching && busy}<span class="dim">{fetching}</span>
          {:else if store.confirmations.length}<span class="dim">{t.waiting(store.confirmations.length)}</span>
          {:else if store.unread}<span class="dim">{t.unread(store.unread)}</span>{/if}
        </div>
      </div>

      {#if last.you || last.yoru}
        <div class="exchange">
          {#if last.you}<p class="you">{last.you.text}</p>{/if}
          {#if last.yoru}<p class="yoru">{last.yoru.text}</p>{/if}
        </div>
      {:else}
        <p class="dim empty">{t.empty}</p>
      {/if}

      {#each store.confirmations as card (card.id)}<ConfirmCard {card} />{/each}

      <Composer />

      <div class="actions">
        <button
          class="btn talk"
          class:on={held || voice === "listening"}
          onpointerdown={(e) => (e.currentTarget.setPointerCapture(e.pointerId), talk(true))}
          onpointerup={() => talk(false)}
          onpointercancel={() => talk(false)}
          onkeydown={(e) => e.key === " " && !e.repeat && (e.preventDefault(), talk(true))}
          onkeyup={(e) => e.key === " " && talk(false)}
        >
          {t.hold} <kbd>{t.key}</kbd>
        </button>
        {#if busy}<button class="btn quiet" onclick={() => call("notch_hush").catch(() => {})}>{t.stop}</button>{/if}
        <button class="btn quiet" onclick={() => call("show_app").catch(() => {})}>{t.open}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  :global(html.notch),
  :global(html.notch body) {
    background: transparent;
    overflow: hidden;
  }

  .shape {
    position: absolute;
    top: 0;
    left: 50%;
    width: var(--w);
    transform: translateX(-50%);
    background: #000;
    color: var(--pearl);
    border-radius: 0 0 calc(var(--bar) * 0.5) calc(var(--bar) * 0.5);
    transition:
      width 0.32s cubic-bezier(0.2, 0.9, 0.3, 1.15),
      border-radius 0.32s ease;
    overflow: hidden;
    box-shadow: 0 0 0 0 transparent;
  }
  .shape.open {
    border-radius: 0 0 26px 26px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.45);
  }
  /* Without a camera notch it sits on the menu bar, a little shorter. */
  .shape.soft:not(.open) {
    border-radius: 0 0 14px 14px;
  }

  .top {
    display: flex;
    justify-content: space-between;
    height: var(--bar);
  }
  .ear {
    width: var(--ear);
    display: grid;
    place-items: center;
  }

  .bars {
    display: flex;
    gap: 2px;
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
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--pearl);
  }
  .dot.thinking {
    background: #4aa3ff;
    animation: pulse 1s ease-in-out infinite;
  }
  .dot.speaking {
    background: #40e080;
  }
  .dot.error {
    background: #ff4d4d;
  }
  .count {
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--ink-3);
    font: 600 10px/16px var(--sans);
    text-align: center;
  }
  .count.wait {
    background: #ff9f43;
    color: #000;
  }

  .panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 6px 18px 18px;
    max-height: 440px;
    overflow-y: auto;
    animation: in 0.25s ease both;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .state {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .state b {
    font: 600 16px/1.3 var(--display, var(--sans));
  }
  .exchange {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .exchange p {
    margin: 0;
    padding: 8px 12px;
    border-radius: 14px;
    max-width: 88%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 6;
    line-clamp: 6;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .you {
    align-self: flex-end;
    background: var(--ink-3);
  }
  .yoru {
    align-self: flex-start;
    background: var(--ink-2);
    border: 1px solid var(--rim);
  }
  .empty {
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .talk {
    flex: 1;
    user-select: none;
  }
  .talk.on {
    background: #40e080;
    color: #000;
  }
  kbd {
    margin-left: 6px;
    font: 500 11px var(--mono);
    opacity: 0.7;
  }

  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(-6px);
    }
  }
</style>
