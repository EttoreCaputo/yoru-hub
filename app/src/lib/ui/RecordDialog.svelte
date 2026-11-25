<!--
  What a recording takes in (the Meetings page's New meeting menu): what the
  computer plays (all of it, one app's sound, or nothing) and which
  microphones, each a track of its own. To record now it also asks a title
  and has the button; to pick the sources only, it's what the window bar's
  Record will take. The choice is kept in the store either way.
-->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { page } from "../meetings-page.svelte";
  import { store } from "../store.svelte";
  import type { MeetingSources } from "../types";
  import Dialog from "./Dialog.svelte";
  import Toggle from "./Toggle.svelte";

  const t = $derived(i18n.t.meetings);
  const mode = $derived(page.dialog?.mode ?? "record");
  let sources = $state<MeetingSources | null>(null);
  let error = $state("");

  $effect(() => {
    if (page.dialog?.title) store.recordTitle = page.dialog.title;
    void readSources();
  });

  async function readSources() {
    try {
      sources = await call<MeetingSources>("meeting_sources");
      if (!store.recordMics) store.recordMics = sources.microphones.filter((m) => m.default).map((m) => m.id);
      const played = store.recordPlayed;
      if (played !== "system" && played !== "none" && !sources.apps.some((a) => a.id === played)) store.recordPlayed = "system";
    } catch (e) {
      error = why(e);
    }
  }

  const mics = $derived(store.recordMics ?? []);
  const toggleMic = (id: string, on: boolean) => (store.recordMics = on ? [...mics, id] : mics.filter((m) => m !== id));
  const played = $derived(store.recordPlayed);

  async function record() {
    await store.record();
    if (store.recording) {
      page.picked = { meeting: store.recording.id };
      page.dialog = null;
    }
  }
</script>

<Dialog title={mode === "record" ? t.recordNow : t.sources} onclose={() => (page.dialog = null)}>
  <div class="record">
    {#if mode === "record"}
      <input class="title" type="text" placeholder={t.titlePlaceholder} bind:value={store.recordTitle} aria-label={t.titlePlaceholder} />
    {/if}
    <div class="setting">
      <div class="what"><b>{t.played}</b><span>{t.playedHint}</span></div>
      <div class="how"><button class="btn small quiet" onclick={readSources}>{t.refresh}</button></div>
    </div>
    <div class="tiles-row" role="radiogroup" aria-label={t.played}>
      <button class="tile inset source" class:on={played === "system"} role="radio" aria-checked={played === "system"} onclick={() => (store.recordPlayed = "system")}>
        <span class="glyph"><Icon name="monitor" size={22} stroke={1.6} /></span>
        <span class="label">{t.everything}</span>
      </button>
      {#each sources?.apps ?? [] as app (app.id)}
        <button class="tile inset source" class:on={played === app.id} role="radio" aria-checked={played === app.id} title={app.name} onclick={() => (store.recordPlayed = app.id)}>
          {#if app.icon}
            <img src="data:image/png;base64,{app.icon}" alt="" />
          {:else}
            <span class="glyph letter">{app.name.slice(0, 1)}</span>
          {/if}
          {#if app.playing}<span class="playing" title={t.playingNow}>♪</span>{/if}
          <span class="label">{app.name}</span>
        </button>
      {/each}
      <button class="tile inset source" class:on={played === "none"} role="radio" aria-checked={played === "none"} onclick={() => (store.recordPlayed = "none")}>
        <span class="glyph"><Icon name="ban" size={22} stroke={1.6} /></span>
        <span class="label">{t.nothing}</span>
      </button>
    </div>
    {#each sources?.microphones ?? [] as mic (mic.id)}
      <div class="setting">
        <div class="what"><b>{mic.name}</b><span>{mic.default ? t.micDefault : t.mic}</span></div>
        <div class="how"><Toggle checked={mics.includes(mic.id)} label={mic.name} onchange={(on) => toggleMic(mic.id, on)} /></div>
      </div>
    {/each}
    <p class="faint note">{mode === "record" ? t.consent : `${t.sourcesHint} ${t.consent}`}</p>
    {#if error}<p class="error-text">{error}</p>{/if}
    {#if store.recordingError}<p class="error-text">{store.recordingError.message}</p>{/if}
  </div>
  {#snippet actions()}
    {#if mode === "record"}
      <button class="btn quiet" onclick={() => (page.dialog = null)}>{t.cancel}</button>
      <button class="btn primary" disabled={!!store.recording || !!store.recordBusy} onclick={record}><span class="rec"></span>{store.recordBusy === "starting" ? t.starting : t.record}</button>
    {:else}
      <button class="btn primary" onclick={() => (page.dialog = null)}>{t.done}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .record {
    width: min(520px, 70vw);
  }
  .title {
    width: 100%;
    box-sizing: border-box;
    margin-bottom: 8px;
  }
  .tiles-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--rim);
  }
  .source {
    position: relative;
    justify-items: center;
    gap: 6px;
    width: 84px;
    padding: 10px 6px 8px;
    border-color: transparent;
    background: transparent;
    font: 500 var(--fs-small)/1.2 var(--sans);
  }
  .source.on {
    border-color: var(--rim-2);
  }
  .source img,
  .glyph {
    width: 40px;
    height: 40px;
  }
  .glyph {
    display: grid;
    place-items: center;
    border-radius: 10px;
    background: var(--ink-3);
    color: var(--pearl-mid);
  }
  .letter {
    font: 600 19px/1 var(--display);
  }
  .label {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .playing {
    position: absolute;
    top: 6px;
    right: 12px;
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--done);
    color: var(--eye);
    font-size: var(--fs-small);
  }
  .note {
    font-size: var(--fs-meta);
    margin-top: 8px;
  }
  .rec {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 7px;
    border-radius: 50%;
    background: currentColor;
  }
</style>
