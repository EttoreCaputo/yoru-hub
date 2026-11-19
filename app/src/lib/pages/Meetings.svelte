<!--
  Meetings (docs/meetings.md): record what the computer plays (all of it,
  or one app's sound) and the microphones, each as a track of its own, or
  bring in an audio or video file (the button, or dropped on the window);
  listen again, and read the transcript Parakeet makes on this computer,
  with "you" told apart by track and the others by voice (Nemotron), each
  of them nameable, and the summary (the local model, or the assistant's
  brain): what was said, decided, left to do and left open. A meeting's
  transcript and summary open in a tab of their own (MeetingView.svelte);
  the meetings are listed on Home, their settings are in Settings.
-->
<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  import { untrack } from "svelte";
  import { call, inPreview, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Meeting, MeetingRequest, MeetingSources } from "../types";
  import { tabs } from "../tabs.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import { clock } from "../ui/when";

  const t = $derived(i18n.t.meetings);
  let error = $state("");
  let sources = $state<MeetingSources | null>(null);
  /** `system`, an app's bundle id, or `none`. */
  let played = $state("system");
  let mics = $state<string[]>([]);
  let title = $state("");
  let starting = $state(false);
  let stopping = $state(false);
  let importing = $state(false);
  let dropping = $state(false);

  $effect(() => {
    void readSources();
  });

  // A recording that just finished ends the stopping.
  $effect(() => {
    const done = store.recorded;
    if (!done) return;
    untrack(() => {
      if (done.state !== "recording") stopping = false;
    });
  });

  // Audio or video files dropped on the window become meetings.
  $effect(() => {
    if (inPreview) return;
    let unlisten: (() => void) | undefined;
    let gone = false;
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent(({ payload }) => {
          if (payload.type === "enter" || payload.type === "over") dropping = true;
          else if (payload.type === "leave") dropping = false;
          else {
            dropping = false;
            for (const path of payload.paths) void bringIn(path);
          }
        }),
      )
      .then((stop) => (gone ? stop() : (unlisten = stop)));
    return () => {
      gone = true;
      unlisten?.();
    };
  });

  /** A file as a new meeting: `path`, or one picked in Finder. */
  async function bringIn(path?: string) {
    importing = true;
    error = "";
    try {
      const m = await call<Meeting | null>("meeting_import", { path: path ?? null });
      void store.refreshMeetings();
      if (m) tabs.open({ kind: "meeting", id: m.id });
    } catch (e) {
      error = why(e);
    } finally {
      importing = false;
    }
  }

  async function readSources() {
    try {
      sources = await call<MeetingSources>("meeting_sources");
      if (!mics.length && !store.recording) mics = sources.microphones.filter((m) => m.default).map((m) => m.id);
      if (played !== "system" && played !== "none" && !sources.apps.some((a) => a.id === played)) played = "system";
    } catch (e) {
      error = why(e);
    }
  }

  const toggleMic = (id: string, on: boolean) => (mics = on ? [...mics, id] : mics.filter((m) => m !== id));
  const nothing = $derived(played === "none" && !mics.length);

  async function record() {
    starting = true;
    error = "";
    store.recordingError = null;
    const request: MeetingRequest = {
      system: played === "system",
      app: played === "system" || played === "none" ? null : played,
      microphones: mics,
      title: title.trim() || null,
    };
    try {
      store.recording = await call<Meeting>("meeting_start", { request });
      title = "";
    } catch (e) {
      error = why(e);
    } finally {
      starting = false;
    }
  }

  async function stop() {
    stopping = true;
    try {
      await call("meeting_stop");
    } catch (e) {
      stopping = false;
      error = why(e);
    }
  }

  const when = (at: number) => `${i18n.date(at)} · ${i18n.time(at)}`;
  const name = (m: Meeting) => m.title ?? t.untitled(when(m.started_at));
  const elapsed = $derived(store.recording ? (store.now - store.recording.started_at) / 1000 : 0);
</script>

<div class="page meetings">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    <div class="actions">
      <button class="btn" disabled={importing} onclick={() => bringIn()}>{importing ? t.importing : t.import}</button>
    </div>
  </div>

  {#if dropping}<div class="drop" aria-hidden="true"><span>{t.dropHere}</span></div>{/if}

  <section class="panel recorder" class:live={!!store.recording}>
    {#if store.recording}
      <div class="live-head">
        <span class="rec" aria-hidden="true"></span>
        <b>{name(store.recording)}</b>
        <span class="mono time">{clock(elapsed)}</span>
        <button class="btn primary stop" disabled={stopping} onclick={stop}>{stopping ? t.stopping : t.stop}</button>
      </div>
      <div class="meters">
        {#each store.recording.tracks as track (track.name)}
          <div class="meter">
            <span class="dim">{track.kind === "mic" ? t.you : t.others} · {track.source}</span>
            <span class="meter level" style:--p={Math.sqrt(store.levels[track.name] ?? 0)}><span></span></span>
          </div>
        {/each}
      </div>
    {:else}
      <div class="played">
        <div class="setting">
          <div class="what"><b>{t.played}</b><span>{t.playedHint}</span></div>
          <div class="how"><button class="btn small quiet" onclick={readSources}>{t.refresh}</button></div>
        </div>
        <div class="tiles-row" role="radiogroup" aria-label={t.played}>
          <button class="tile inset source" class:on={played === "system"} role="radio" aria-checked={played === "system"} onclick={() => (played = "system")}>
            <span class="glyph"><Icon name="monitor" size={24} stroke={1.6} /></span>
            <span class="label">{t.everything}</span>
          </button>
          {#each sources?.apps ?? [] as app (app.id)}
            <button class="tile inset source" class:on={played === app.id} role="radio" aria-checked={played === app.id} title={app.name} onclick={() => (played = app.id)}>
              {#if app.icon}
                <img src="data:image/png;base64,{app.icon}" alt="" />
              {:else}
                <span class="glyph letter">{app.name.slice(0, 1)}</span>
              {/if}
              {#if app.playing}<span class="playing" title={t.playingNow}>♪</span>{/if}
              <span class="label">{app.name}</span>
            </button>
          {/each}
          <button class="tile inset source" class:on={played === "none"} role="radio" aria-checked={played === "none"} onclick={() => (played = "none")}>
            <span class="glyph"><Icon name="ban" size={24} stroke={1.6} /></span>
            <span class="label">{t.nothing}</span>
          </button>
        </div>
      </div>
      {#each sources?.microphones ?? [] as mic (mic.id)}
        <div class="setting">
          <div class="what"><b>{mic.name}</b><span>{mic.default ? t.micDefault : t.mic}</span></div>
          <div class="how"><Toggle checked={mics.includes(mic.id)} label={mic.name} onchange={(on) => toggleMic(mic.id, on)} /></div>
        </div>
      {/each}
      <div class="setting">
        <input class="title" type="text" placeholder={t.titlePlaceholder} bind:value={title} aria-label={t.titlePlaceholder} />
        <button class="btn primary" disabled={nothing || starting} onclick={record}>
          <span class="rec small" aria-hidden="true"></span>{starting ? t.starting : t.record}
        </button>
      </div>
      <p class="faint note">{t.consent}</p>
    {/if}
  </section>

  {#if error}<p class="error-text">{error}</p>{/if}
  {#if store.recordingError?.code === "no_system_audio"}
    <p class="error-text">
      {t.noSystemAudio}
      <button class="btn small" onclick={() => call("open_audio_privacy").catch((e) => (error = why(e)))}>{t.openPrivacy}</button>
    </p>
  {:else if store.recordingError?.code === "no_microphone_sound"}
    <p class="error-text">{t.noMicSound}</p>
  {:else if store.recordingError}
    <p class="error-text">{store.recordingError.message}</p>
  {/if}
</div>

<style>
  .small {
    font-size: var(--fs-meta);
  }
  .drop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--night) 70%, transparent);
    border: 2px dashed var(--rim-2);
    pointer-events: none;
  }
  .drop span {
    font: 600 var(--fs-head)/1.2 var(--display);
    color: var(--pearl);
  }
  .recorder.live {
    border-color: color-mix(in srgb, var(--wrong) 40%, var(--rim));
  }
  .live-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .time {
    color: var(--haze);
  }
  .stop {
    margin-left: auto;
  }
  .rec {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--wrong);
    flex-shrink: 0;
  }
  .live .rec {
    animation: pulse 1.4s ease-in-out infinite;
  }
  .rec.small {
    width: 8px;
    height: 8px;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .meters {
    display: grid;
    gap: 10px;
    margin-top: 16px;
  }
  .meter {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 220px;
    align-items: center;
    gap: 14px;
    font-size: var(--fs-meta);
  }
  .level {
    height: 6px;
  }
  .level > span {
    transition-duration: 200ms;
    transition-timing-function: linear;
  }
  .played {
    padding-bottom: 14px;
    border-bottom: 1px solid var(--rim);
  }
  .played .setting {
    padding-top: 2px;
  }
  .tiles-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .source {
    position: relative;
    justify-items: center;
    gap: 6px;
    width: 88px;
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
    width: 44px;
    height: 44px;
  }
  .glyph {
    display: grid;
    place-items: center;
    border-radius: 11px;
    background: var(--ink-3);
    color: var(--pearl-mid);
  }
  .letter {
    font: 600 21px/1 var(--display);
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
    right: 14px;
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--done);
    color: var(--eye);
    font-size: var(--fs-small);
  }
  .title {
    flex: 1;
    max-width: 360px;
  }
  .note {
    font-size: var(--fs-meta);
    margin-top: 8px;
  }
</style>
