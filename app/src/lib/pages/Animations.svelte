<!--
  Every animation clip the board shown has, as its `debug/clips` lists
  them, to play on it one at a time or all in a row: mirrored, or in the
  colours of a service, if asked. Read again whenever the board changes.
  Each has a preview drawn in the app from the bundled firmware's frames,
  played while the pointer is on it; with no board, the bundled clips show.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { BoardClip } from "../types";
  import Toggle from "../ui/Toggle.svelte";
  import ClipPreview from "../ui/ClipPreview.svelte";
  import { DEFAULT_RGB, loadClips, serviceRgb, type Clip } from "../clips";

  const t = $derived(i18n.t.animations);
  /** The services whose colour and icon a clip can take, as the firmware names them. */
  const SERVICES = ["Gmail", "Calendar", "Drive", "GitHub", "Slack", "Spotify", "Web", "Memory", "Tasks", "Timers"];
  /** A clip's own length leaves the eyes no time to settle: this much more before the next. */
  const GAP_S = 0.6;

  let clips = $state<BoardClip[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let mirror = $state(false);
  let source = $state("");
  /** The clip playing, as far as the page knows: a looping one until stopped. */
  let current = $state<string | null>(null);
  let playingAll = $state(false);
  // Bumped to drop a running "play all" and a pending "it ended".
  let sequence = 0;
  let ending: ReturnType<typeof setTimeout> | undefined;

  const live = $derived(store.boardState === "live");
  const once = $derived(clips.filter((c) => !c.loop));

  // The previews: the bundled firmware's frames, by name.
  let drawn = $state(new Map<string, Clip>());
  $effect(() => {
    loadClips().then((m) => (drawn = m));
  });
  /** The board's clips, or the bundled ones while there's no board. */
  const shown = $derived<BoardClip[]>(
    live ? clips : [...drawn.values()].map((c) => ({ name: c.name, loop: c.loop, seconds: c.frames.length / c.fps })),
  );
  const rgb = $derived(source ? serviceRgb(source) : DEFAULT_RGB);
  let hovered = $state<string | null>(null);

  $effect(() => {
    void store.boardId;
    if (live) untrack(load);
    return cancel;
  });

  async function load() {
    loaded = false;
    error = "";
    clips = [];
    try {
      clips = await call<BoardClip[]>("yoru_clips", { board: null });
    } catch (e) {
      error = why(e);
    } finally {
      loaded = true;
    }
  }


  /** "wake_stretch" → "Wake stretch". */
  const readable = (name: string) => {
    const words = name.replace(/_/g, " ");
    return words.charAt(0).toUpperCase() + words.slice(1);
  };

  async function play(c: BoardClip) {
    error = "";
    clearTimeout(ending);
    current = c.name;
    try {
      await call("yoru_play_clip", { board: null, name: c.name, mirror, source: source || null });
    } catch (e) {
      current = null;
      error = why(e);
      return false;
    }
    // A looping clip stays current until Stop; the others end on their own.
    if (!c.loop) ending = setTimeout(() => current === c.name && (current = null), c.seconds * 1000);
    return true;
  }

  /** One clip, now: it ends a "play all" that runs. */
  function playOne(c: BoardClip) {
    cancel();
    void play(c);
  }

  /** Every clip that ends on its own, in turn, until done or stopped. */
  async function playAll() {
    cancel();
    const run = sequence;
    playingAll = true;
    for (const c of once) {
      if (run !== sequence || !(await play(c))) break;
      await new Promise((r) => setTimeout(r, (c.seconds + GAP_S) * 1000));
    }
    if (run === sequence) playingAll = false;
  }

  function cancel() {
    sequence++;
    playingAll = false;
    clearTimeout(ending);
  }

  async function stop() {
    cancel();
    current = null;
    try {
      await call("yoru_stop_clip", { board: null });
    } catch (e) {
      error = why(e);
    }
  }
</script>

<div class="page animations">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  {#if !live}
    <p class="faint note">{t.offlinePreview}</p>
  {/if}
  <div class="bar">
    <label class="option">
      <Toggle checked={mirror} label={t.mirror} onchange={(on) => (mirror = on)} />
      <span>{t.mirror}</span>
    </label>
    <label class="option">
      <span>{t.service}</span>
      <select class="compact" bind:value={source} aria-label={t.service}>
        <option value="">{t.none}</option>
        {#each SERVICES as s (s)}<option value={s}>{s}</option>{/each}
      </select>
    </label>
    <span class="faint small">{t.count(shown.length)}</span>
    {#if live}
      <span class="end">
        {#if playingAll}
          <button class="btn small" onclick={stop}>{t.stopAll}</button>
        {:else}
          <button class="btn small" disabled={!once.length} onclick={playAll}>{t.playAll}</button>
        {/if}
        <button class="btn small" disabled={!current && !playingAll} onclick={stop}>{t.stop}</button>
      </span>
    {/if}
  </div>

  {#if error}<p class="error-text">{error}</p>{/if}

  {#if shown.length}
    <div class="grid">
      {#each shown as c (c.name)}
        {@const preview = drawn.get(c.name)}
        <article
          class="clip"
          class:on={current === c.name}
          onpointerenter={() => (hovered = c.name)}
          onpointerleave={() => hovered === c.name && (hovered = null)}
        >
          <div class="stage">
            {#if preview}
              <ClipPreview clip={preview} playing={hovered === c.name || current === c.name} {rgb} {mirror} />
            {:else}
              <div class="blank"></div>
            {/if}
            {#if live}
              <button class="icon-btn round filled play" class:on={current === c.name} onclick={() => playOne(c)} aria-label={t.play} title={t.play}>
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5.5v13l10.5-6.5z" /></svg>
              </button>
            {/if}
          </div>
          <span class="name">{readable(c.name)}</span>
          <span class="facts">
            {#if current === c.name}<span class="tag done">{t.playing}</span>{/if}
            {#if c.loop}<span class="tag">{t.loops}</span>{/if}
            <span class="faint">{c.seconds.toFixed(1)} s</span>
          </span>
        </article>
      {/each}
    </div>
  {:else if live && loaded && !error}
    <div class="empty">
      <b>{t.empty}</b>
      <p>{t.emptyHint}</p>
    </div>
  {:else if !live && !drawn.size}
    <div class="empty">
      <b>{t.offline}</b>
      <p>{t.offlineHint}</p>
    </div>
  {/if}
</div>

<style>
  .small {
    font-size: var(--fs-meta);
  }
  .bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 14px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--rim);
    margin-bottom: 4px;
  }
  .option {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--haze);
    font-size: var(--fs-body);
  }
  .end {
    display: flex;
    gap: 8px;
    margin-left: auto;
  }
  .note {
    margin: 0 0 10px;
    font-size: var(--fs-meta);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 12px;
    padding-top: 12px;
  }
  .clip {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 14px 10px 12px;
    border: 1px solid var(--rim);
    border-radius: var(--r-lg);
    transition: border-color 150ms ease, background 150ms ease;
  }
  .clip:hover,
  .clip.on {
    border-color: var(--rim-2);
    background: var(--hover);
  }
  .stage {
    position: relative;
  }
  .blank {
    width: 132px;
    height: 132px;
    border-radius: 50%;
    background: #000;
  }
  .play {
    position: absolute;
    right: -4px;
    bottom: -4px;
    opacity: 0;
    transition: opacity 150ms ease;
  }
  .clip:hover .play,
  .play:focus-visible,
  .play.on {
    opacity: 1;
  }
  .name {
    color: var(--pearl);
    font-size: var(--fs-body);
    text-align: center;
  }
  .facts {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-meta);
  }
</style>
