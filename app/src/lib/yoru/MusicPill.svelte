<!-- What plays on this computer: Yoru bobs to it, and the pill drives the player. -->
<script lang="ts">
  import Pill from "./Pill.svelte";
  import { monitor, type MusicAction } from "../monitor.svelte";
  import { i18n } from "../i18n/index.svelte";

  const t = $derived(i18n.t.yoru.music);
  const clock = $derived(i18n.t.yoru.clock);
  const m = $derived(monitor.last?.music);
  const playing = $derived(m?.state === "play");
  // The position moves on between snapshots while it plays.
  const pos = $derived(m?.pos_s === undefined ? undefined : Math.min(m.dur_s ?? Infinity, m.pos_s + (playing ? Math.max(0, monitor.now - monitor.sentAt) / 1000 : 0)));
  const send = (a: MusicAction) => void monitor.musicControl(a).catch(() => {});
</script>

<Pill title={t.title} accent="#40e080" chip={playing ? t.dances : null} chipLive={playing}>
  {#if m}
    <div class="track">
      <div class="disc" class:spin={playing} aria-hidden="true"><i></i></div>
      <div class="words">
        <span class="title">{m.title}</span>
        {#if m.artist}<span class="artist">{m.artist}</span>{/if}
      </div>
    </div>
    {#if pos !== undefined && m.dur_s}
      <div class="progress">
        <span>{clock(pos)}</span>
        <div class="bar"><div style:width="{(pos / m.dur_s) * 100}%"></div></div>
        <span>{clock(m.dur_s)}</span>
      </div>
    {/if}
    <div class="controls">
      <button aria-label={t.previous} title={t.previous} onclick={() => send("previous")}>
        <svg viewBox="0 0 24 24"><path d="M7 6v12M18 6l-8 6 8 6z" /></svg>
      </button>
      <button class="main" aria-label={playing ? t.pause : t.play} title={playing ? t.pause : t.play} onclick={() => send("toggle")}>
        {#if playing}
          <svg viewBox="0 0 24 24"><path d="M9 6v12M15 6v12" /></svg>
        {:else}
          <svg viewBox="0 0 24 24"><path d="M8 5.5v13l10.5-6.5z" /></svg>
        {/if}
      </button>
      <button aria-label={t.next} title={t.next} onclick={() => send("next")}>
        <svg viewBox="0 0 24 24"><path d="M17 6v12M6 6l8 6-8 6z" /></svg>
      </button>
    </div>
  {:else}
    <p class="empty">{t.none}</p>
  {/if}
</Pill>

<style>
  .track {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .disc {
    flex: none;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background:
      radial-gradient(circle, transparent 28%, rgba(255, 255, 255, 0.06) 29%, transparent 31%, rgba(255, 255, 255, 0.05) 50%, transparent 52%),
      conic-gradient(from 30deg, #20352a, #40e080 18%, #142019 35%, #1d3328 60%, #40e080 78%, #142019);
    box-shadow: 0 0 18px -6px #40e080;
  }
  .disc i {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--bg);
  }
  .disc.spin {
    animation: spin 3.2s linear infinite;
  }
  .words {
    display: grid;
    gap: 5px;
    min-width: 0;
  }
  .title,
  .artist {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .title {
    font: 600 14px/1.1 var(--sans);
  }
  .artist {
    font: 450 12px/1.1 var(--sans);
    color: var(--dim);
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 500 10.5px/1 var(--mono);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .bar {
    flex: 1;
    height: 4px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.07);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: #40e080;
    border-radius: inherit;
    transition: width 300ms linear;
  }
  .controls {
    display: flex;
    justify-content: center;
    gap: 10px;
  }
  .controls button {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.03);
    color: var(--dim);
    cursor: pointer;
    transition:
      background 160ms,
      color 160ms;
  }
  .controls button:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  .controls .main {
    width: 40px;
    height: 40px;
    background: #40e080;
    border-color: transparent;
    color: #04130a;
  }
  .controls .main:hover {
    background: #5cf096;
    color: #04130a;
  }
  svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .main svg {
    fill: currentColor;
  }
  .empty {
    margin: 0;
    font: 450 12.5px/1.45 var(--sans);
    color: var(--faint);
  }
  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }
</style>
