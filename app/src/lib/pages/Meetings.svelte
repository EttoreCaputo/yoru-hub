<!--
  Meetings (docs/meetings.md): record what the computer plays (all of it,
  or one app's sound) and the microphones, each as a track of its own, or
  bring in an audio or video file (the button, or dropped on the window);
  listen again, and read the transcript Parakeet makes on this computer,
  with "you" told apart by track and the others by voice (Nemotron), each
  of them nameable, and the summary (the local model, or the assistant's
  brain): what was said, decided, left to do and left open. A meeting's
  transcript and summary open in a tab of their own (MeetingView.svelte).
  The meetings' settings are a panel of this page.
-->
<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  import { untrack } from "svelte";
  import { call, inPreview, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Meeting, MeetingRequest, MeetingSettings, MeetingSources, MeetingTrack, PocketSettings, Summarizers } from "../types";
  import { tabs } from "../tabs.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import PocketPanel from "../ui/PocketPanel.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import { clock } from "../ui/when";

  const t = $derived(i18n.t.meetings);
  let meetings = $state<Meeting[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let sources = $state<MeetingSources | null>(null);
  /** `system`, an app's bundle id, or `none`. */
  let played = $state("system");
  let mics = $state<string[]>([]);
  let title = $state("");
  let starting = $state(false);
  let stopping = $state(false);
  /** Blob URLs of the tracks loaded for listening, by `<id>/<track>`. */
  let audio = $state<Record<string, string>>({});
  let editing = $state<string | null>(null);
  let draft = $state("");
  let showSettings = $state(false);
  let settings = $state<MeetingSettings | null>(null);
  /** Whether the transcription model is downloaded; null until known. */
  let model = $state<boolean | null>(null);
  let importing = $state(false);
  let dropping = $state(false);
  let summarizers = $state<Summarizers | null>(null);
  let fetchingLlm = $state(false);

  $effect(() => {
    call<Meeting[]>("meetings_list").then(
      (m) => ((meetings = m), (loaded = true)),
      (e) => (error = why(e)),
    );
    void readSources();
    call<MeetingSettings>("meeting_settings").then((x) => (settings = x), () => {});
    call<boolean>("meeting_model").then((x) => (model = x), () => {});
    call<Summarizers>("meeting_summarizers").then((x) => (summarizers = x), () => {});
    return () => Object.values(audio).forEach((url) => URL.revokeObjectURL(url));
  });

  // A recording that just finished, or a transcript just made, updates the list.
  $effect(() => {
    const done = store.recorded;
    if (!done) return;
    untrack(() => {
      if (done.state !== "recording") stopping = false;
      meetings = [done, ...meetings.filter((m) => m.id !== done.id)].sort((a, b) => b.id.localeCompare(a.id));
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
      if (m) meetings = [m, ...meetings.filter((x) => x.id !== m.id)];
      void store.refreshMeetings();
    } catch (e) {
      error = why(e);
    } finally {
      importing = false;
    }
  }

  async function saveSettings(change: Partial<MeetingSettings>) {
    if (!settings) return;
    try {
      settings = await call<MeetingSettings>("set_meeting_settings", { settings: { ...settings, ...change } });
    } catch (e) {
      error = why(e);
    }
  }

  async function savePocket(change: Partial<PocketSettings>) {
    if (!store.pocket) return;
    try {
      await call("set_pocket_settings", { settings: { ...store.pocket.settings, ...change } });
    } catch (e) {
      error = why(e);
    }
  }

  async function downloadModel() {
    error = "";
    store.modelDownload = 0;
    try {
      await call("meeting_model_download");
      model = await call<boolean>("meeting_model");
    } catch (e) {
      error = why(e);
    } finally {
      store.modelDownload = null;
    }
  }

  async function transcribe(m: Meeting) {
    error = "";
    store.transcribing = { ...store.transcribing, [m.id]: 0 };
    try {
      await call("meeting_transcribe", { id: m.id });
    } catch (e) {
      error = why(e);
    }
  }

  /** The models to summarize with, by provider. */
  const providers = $derived.by(() => {
    const by = new Map<string, Summarizers["models"]>();
    for (const m of summarizers?.models ?? []) by.set(m.provider, [...(by.get(m.provider) ?? []), m]);
    return [...by];
  });
  /** The chosen model when its provider's key is gone: still shown, as missing. */
  const lost = $derived(!!settings && !!summarizers && !["local", "brain"].includes(settings.summary_by) && !summarizers.models.some((m) => m.id === settings!.summary_by));

  /** Qwen3.5 4B, the local model, through the voice's downloads. */
  async function fetchLlm() {
    fetchingLlm = true;
    error = "";
    try {
      await call("download_model", { id: "qwen3.5-4b" });
      summarizers = await call<Summarizers>("meeting_summarizers");
    } catch (e) {
      error = why(e);
    } finally {
      fetchingLlm = false;
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

  async function listen(m: Meeting, track: MeetingTrack) {
    const key = `${m.id}/${track.name}`;
    if (audio[key]) return;
    error = "";
    try {
      const raw = await call<ArrayBuffer | Uint8Array | number[]>("meeting_audio", { id: m.id, track: track.name });
      const bytes = raw instanceof ArrayBuffer ? raw : ArrayBuffer.isView(raw) ? raw : Uint8Array.from(raw);
      const type = track.file?.endsWith(".caf") ? "audio/x-caf" : "audio/mp4";
      audio = { ...audio, [key]: URL.createObjectURL(new Blob([bytes as BlobPart], { type })) };
    } catch (e) {
      error = why(e);
    }
  }

  function edit(m: Meeting) {
    editing = m.id;
    draft = m.title ?? "";
  }

  async function rename(m: Meeting) {
    editing = null;
    if ((m.title ?? "") === draft.trim()) return;
    try {
      const renamed = await call<Meeting>("rename_meeting", { id: m.id, title: draft });
      meetings = meetings.map((x) => (x.id === m.id ? renamed : x));
      void store.refreshMeetings();
    } catch (e) {
      error = why(e);
    }
  }

  async function remove(m: Meeting) {
    try {
      meetings = await call<Meeting[]>("delete_meeting", { id: m.id });
      void store.refreshMeetings();
      for (const track of m.tracks) {
        const key = `${m.id}/${track.name}`;
        if (audio[key]) URL.revokeObjectURL(audio[key]);
      }
    } catch (e) {
      error = why(e);
    }
  }

  const when = (at: number) => `${i18n.date(at)} · ${i18n.time(at)}`;
  const name = (m: Meeting) => m.title ?? t.untitled(when(m.started_at));
  const elapsed = $derived(store.recording ? (store.now - store.recording.started_at) / 1000 : 0);
  const percent = (f: number) => Math.round(f * 100);
  const LANGUAGES = ["", "it", "en", "es", "fr", "de", "pt"];
</script>

<div class="page meetings">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    <div class="actions">
      <button class="btn" disabled={importing} onclick={() => bringIn()}>{importing ? t.importing : t.import}</button>
      <button class="btn" class:on={showSettings} aria-expanded={showSettings} onclick={() => (showSettings = !showSettings)}>{t.settings}</button>
    </div>
  </div>

  {#if showSettings && settings}
    <section class="panel settings">
      <div class="setting">
        <div class="what"><b>{t.detect}</b><span>{t.detectHint}</span></div>
        <div class="how"><Toggle checked={settings.detect} label={t.detect} onchange={(on) => saveSettings({ detect: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.when}</b><span>{t.whenHint}</span></div>
        <div class="how chips" role="radiogroup" aria-label={t.when}>
          {#each [["after", t.whenAfter], ["live", t.whenLive], ["manual", t.whenManual]] as [id, label] (id)}
            <button
              class="chip"
              class:on={settings.transcribe === id}
              role="radio"
              aria-checked={settings.transcribe === id}
              disabled={id === "live"}
              title={id === "live" ? t.liveSoon : undefined}
              onclick={() => saveSettings({ transcribe: id as MeetingSettings["transcribe"] })}>{label}</button
            >
          {/each}
        </div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.language}</b><span>{t.languageHint}</span></div>
        <div class="how">
          <select value={settings.language} onchange={(e) => saveSettings({ language: e.currentTarget.value })} aria-label={t.language}>
            {#each LANGUAGES as code (code)}<option value={code}>{code ? t.languages[code as keyof typeof t.languages] : t.languageAuto}</option>{/each}
          </select>
        </div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.speakers}</b><span>{t.speakersHint}</span></div>
        <div class="how"><Toggle checked={settings.speakers} label={t.speakers} onchange={(on) => saveSettings({ speakers: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.summaryBy}</b><span>{t.summaryByHint}</span></div>
        <div class="how">
          <select value={settings.summary_by} onchange={(e) => saveSettings({ summary_by: e.currentTarget.value })} aria-label={t.summaryBy}>
            <option value="local">{t.thisComputer}</option>
            <option value="brain" disabled={!summarizers?.brain}>{summarizers?.brain ? `${t.brain} (${summarizers.brain})` : t.noBrain}</option>
            {#each providers as [provider, models] (provider)}
              <optgroup label={provider}>
                {#each models as m (m.id)}<option value={m.id} title={m.note || m.model}>{m.note.split(":")[0] || m.model}</option>{/each}
              </optgroup>
            {/each}
            {#if lost}<option value={settings.summary_by}>{settings.summary_by} ({t.noKey})</option>{/if}
          </select>
        </div>
      </div>
      {#if settings.summary_by === "local" && summarizers && !summarizers.local}
        <div class="setting">
          <div class="what"><b>Qwen3.5 4B</b><span>{t.llmHint}</span></div>
          <div class="how"><button class="btn small primary" disabled={fetchingLlm} onclick={fetchLlm}>{fetchingLlm ? t.fetching : t.download}</button></div>
        </div>
      {/if}
      <div class="setting">
        <div class="what"><b>{t.autoSummary}</b><span>{t.autoSummaryHint}</span></div>
        <div class="how"><Toggle checked={settings.auto_summary} label={t.autoSummary} onchange={(on) => saveSettings({ auto_summary: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.model}</b><span>{t.modelHint}</span></div>
        <div class="how">
          {#if store.modelDownload !== null}
            <span class="dim small">{t.downloading(percent(store.modelDownload))}</span>
          {:else if model}
            <span class="dim small"><span class="dot done"></span> {t.modelReady}</span>
          {:else if model === false}
            <button class="btn small primary" onclick={downloadModel}>{t.download}</button>
          {/if}
        </div>
      </div>
      {#if store.pocket}
        {@const p = store.pocket.settings}
        {@const tp = i18n.t.pocket}
        <h3 class="sub">Pocket</h3>
        <div class="setting">
          <div class="what"><b>{tp.enabled}</b><span>{tp.enabledHint}</span></div>
          <div class="how"><Toggle checked={p.enabled} label={tp.enabled} onchange={(on) => savePocket({ enabled: on })} /></div>
        </div>
        {#if p.enabled}
          <div class="setting">
            <div class="what"><b>{tp.autoSync}</b><span>{tp.autoSyncHint}</span></div>
            <div class="how"><Toggle checked={p.auto_sync} label={tp.autoSync} onchange={(on) => savePocket({ auto_sync: on })} /></div>
          </div>
          <div class="setting">
            <div class="what"><b>{tp.deleteAfter}</b><span>{tp.deleteAfterHint}</span></div>
            <div class="how"><Toggle checked={p.delete_after_sync} label={tp.deleteAfter} onchange={(on) => savePocket({ delete_after_sync: on })} /></div>
          </div>
          <div class="setting">
            <div class="what"><b>{tp.setClock}</b><span>{tp.setClockHint}</span></div>
            <div class="how"><Toggle checked={p.set_clock} label={tp.setClock} onchange={(on) => savePocket({ set_clock: on })} /></div>
          </div>
          {#if store.pocket.phase !== "unpaired"}
            <div class="setting">
              <div class="what"><b>{tp.forget}</b><span>{tp.forgetHint}</span></div>
              <div class="how"><ConfirmButton small label={tp.forget} question={tp.forgetConfirm} sure={tp.forget} onconfirm={() => call("pocket_forget").catch((e) => (error = why(e)))} /></div>
            </div>
          {/if}
        {/if}
      {/if}
    </section>
  {/if}

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

  {#if store.pocket && store.pocket.phase !== "off"}<PocketPanel pocket={store.pocket} />{/if}

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

  <h2 class="section-title">{t.recorded}</h2>
  {#if meetings.length}
    <div class="list">
      {#each meetings as m (m.id)}
        <article class="meeting">
          <header>
            {#if editing === m.id}
              <!-- svelte-ignore a11y_autofocus -->
              <input
                type="text"
                bind:value={draft}
                autofocus
                aria-label={t.rename}
                onblur={() => rename(m)}
                onkeydown={(e) => (e.key === "Enter" ? rename(m) : e.key === "Escape" ? (editing = null) : null)}
              />
            {:else}
              <button class="name" onclick={() => edit(m)} title={t.rename}>{name(m)}</button>
            {/if}
            <span class="faint small">
              {when(m.started_at)} · {clock(m.duration_ms / 1000)}
              {#if m.state === "interrupted"} · <span class="cut">{t.interrupted}</span>{/if}
              {#if m.state === "recording"} · {t.recordingNow}{/if}
            </span>
            {#if m.origin === "pocket"}<span class="tag">Pocket</span>{/if}
            {#if m.state !== "recording"}
              <span class="end">
                {#if m.id in store.transcribing}
                  <span class="faint small">{t.transcribing(percent(store.transcribing[m.id]))}</span>
                {:else if m.id in store.summarizing}
                  <span class="faint small">{t.summarizing(percent(store.summarizing[m.id]))}</span>
                {:else if m.transcribed_at}
                  <button class="btn small" onclick={() => tabs.open({ kind: "meeting", id: m.id })}>{t.transcript}</button>
                {:else if m.tracks.some((x) => x.file)}
                  <button class="btn small" disabled={!model} title={model ? undefined : t.needModel} onclick={() => transcribe(m)}>{t.transcribe}</button>
                {/if}
                <ConfirmButton small label={t.delete} question={t.deleteConfirm} sure={t.delete} onconfirm={() => remove(m)} />
              </span>
            {/if}
          </header>
          {#if store.transcribeErrors[m.id]}<p class="error-text">{t.failed(store.transcribeErrors[m.id])}</p>{/if}
          {#if store.summaryErrors[m.id]}<p class="error-text">{t.summaryFailed(store.summaryErrors[m.id])}</p>{/if}
          {#each m.tracks as track (track.name)}
            {@const key = `${m.id}/${track.name}`}
            <div class="track">
              <span class="dim small">{track.kind === "mic" ? t.you : t.others} · {track.source}</span>
              {#if audio[key]}
                <audio controls src={audio[key]}></audio>
              {:else if track.file}
                <button class="btn small" onclick={() => listen(m, track)}>{t.listen}</button>
              {:else}
                <span class="faint small">{t.noFile}</span>
              {/if}
            </div>
          {/each}
        </article>
      {/each}
    </div>
  {:else if loaded}
    <div class="empty">
      <b>{t.empty}</b>
      <p>{t.emptyHint}</p>
    </div>
  {/if}
</div>

<style>
  .small {
    font-size: var(--fs-meta);
  }
  .btn.on {
    background: var(--ink-3);
  }
  .settings {
    margin-bottom: 14px;
  }
  .sub {
    margin: 18px 0 4px;
    padding-top: 14px;
    border-top: 1px solid var(--rim);
    font: 600 var(--fs-small)/1.2 var(--sans);
    color: var(--haze);
    text-transform: uppercase;
    letter-spacing: 0.06em;
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
  .meeting {
    padding: 14px 4px;
  }
  .meeting + .meeting {
    border-top: 1px solid var(--rim);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .name {
    all: unset;
    cursor: text;
    font-weight: 600;
    color: var(--pearl);
  }
  .end {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
  }
  .cut {
    color: var(--wait);
  }
  .track {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
    margin-top: 8px;
    min-height: 32px;
  }
  audio {
    height: 32px;
    width: 300px;
  }
</style>
