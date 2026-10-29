<!--
  Meetings (docs/meetings.md): record what the computer plays (all of it,
  or one app's sound) and the microphones, each as a track of its own, or
  bring in an audio or video file (the button, or dropped on the window);
  listen again, and read the transcript Parakeet makes on this computer,
  with "you" told apart by track and the others by voice (Nemotron), each
  of them nameable, and the summary (the local model, or the assistant's
  brain): what was said, decided, left to do and left open. The meetings' settings are a panel of this page.
-->
<script lang="ts">
  import { tick, untrack } from "svelte";
  import { call, inPreview, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Meeting, MeetingRequest, MeetingSettings, MeetingSources, MeetingTrack, Speaker, Summarizers, Summary, Transcript, TranscriptLine } from "../types";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
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
  /** The meeting whose transcript is open. */
  let open = $state<string | null>(null);
  let transcripts = $state<Record<string, Transcript | null>>({});
  let exported = $state("");
  let importing = $state(false);
  let dropping = $state(false);
  /** The tracks' players, to jump to a line. */
  const players: Record<string, HTMLAudioElement> = {};
  let summaries = $state<Record<string, Summary | null>>({});
  let summarizers = $state<Summarizers | null>(null);
  let fetchingLlm = $state(false);
  let copied = $state<string | null>(null);

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
      if (done.transcribed_at && done.id in transcripts) void readTranscript(done.id);
    });
  });

  // A summary just written shows at once.
  $effect(() => {
    const done = store.summarized;
    if (done) untrack(() => (summaries = { ...summaries, [done.id]: done.summary }));
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

  async function readTranscript(id: string) {
    try {
      transcripts = { ...transcripts, [id]: await call<Transcript | null>("meeting_transcript", { id }) };
    } catch (e) {
      error = why(e);
    }
  }

  function toggleTranscript(m: Meeting) {
    exported = "";
    open = open === m.id ? null : m.id;
    if (open && !(m.id in transcripts)) void readTranscript(m.id);
    if (open && !(m.id in summaries)) call<Summary | null>("meeting_summary", { id: m.id }).then((x) => (summaries = { ...summaries, [m.id]: x }), (e) => (error = why(e)));
  }

  /** Whether the settings' summarizer is there. */
  const canSummarize = $derived(!!settings && !!summarizers && (settings.summary_by === "local" ? summarizers.local : !!summarizers.brain));

  async function summarize(m: Meeting) {
    error = "";
    store.summarizing = { ...store.summarizing, [m.id]: 0 };
    try {
      await call("meeting_summarize", { id: m.id });
    } catch (e) {
      error = why(e);
    }
  }

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

  /** The summary as Markdown, on the clipboard. */
  async function copySummary(m: Meeting, s: Summary) {
    const list = (title: string, items: string[]) => (items.length ? `\n## ${title}\n\n${items.map((x) => `- ${x}`).join("\n")}\n` : "");
    const actions = s.actions.map((a) => {
      const who = [a.owner, a.due].map((x) => x.trim()).filter(Boolean).join(", ");
      return who ? `${a.text} (${who})` : a.text;
    });
    const text = `# ${s.title}\n\n${s.overview}\n${list(t.points, s.points)}${list(t.decisions, s.decisions)}${list(t.actions, actions)}${list(t.questions, s.questions)}`;
    try {
      await navigator.clipboard.writeText(text);
      copied = m.id;
      setTimeout(() => (copied = copied === m.id ? null : copied), 1600);
    } catch (e) {
      error = why(e);
    }
  }

  /** Play the line's track from where it starts. */
  async function seek(m: Meeting, line: TranscriptLine) {
    const track = m.tracks.find((x) => x.name === line.track);
    if (!track) return;
    const key = `${m.id}/${track.name}`;
    await listen(m, track);
    await tick();
    const player = players[key];
    if (!player) return;
    player.currentTime = line.start_ms / 1000;
    player.play().catch(() => {});
  }

  /** The speaker being named: `<meeting>/<key>`. */
  let naming = $state<string | null>(null);
  let nameDraft = $state("");

  const speakerOf = (transcript: Transcript, line: TranscriptLine): Speaker | undefined => {
    const key = `${line.track}/${line.speaker ?? "-"}`;
    return transcript.speakers.find((s) => s.key === key);
  };
  const label = (s: Speaker | undefined) => s?.name ?? (!s ? "?" : s.number === 0 ? t.you : t.person(s.number));
  // A color per person, the same in every line.
  const TINTS = ["var(--think)", "#c58cff", "#5fd4c8", "var(--wait)", "#ff7eb6", "var(--done)", "#f5d061", "#8aa4ff"];
  const tint = (s: Speaker | undefined) => (s ? TINTS[s.number % TINTS.length] : "var(--dusk)");

  function startNaming(m: Meeting, s: Speaker) {
    naming = `${m.id}/${s.key}`;
    nameDraft = s.name ?? "";
  }

  async function saveName(m: Meeting, s: Speaker) {
    if (naming !== `${m.id}/${s.key}`) return;
    naming = null;
    if ((s.name ?? "") === nameDraft.trim()) return;
    try {
      transcripts = { ...transcripts, [m.id]: await call<Transcript>("rename_speaker", { id: m.id, key: s.key, name: nameDraft }) };
    } catch (e) {
      error = why(e);
    }
  }

  async function exportAs(m: Meeting, format: "markdown" | "srt" | "text") {
    error = "";
    try {
      exported = await call<string>("meeting_export", { id: m.id, format, you: t.you, person: t.personWord });
    } catch (e) {
      error = why(e);
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
    } catch (e) {
      error = why(e);
    }
  }

  async function remove(m: Meeting) {
    try {
      meetings = await call<Meeting[]>("delete_meeting", { id: m.id });
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
    <div class="head-actions">
      <button class="btn small" disabled={importing} onclick={() => bringIn()}>{importing ? t.importing : t.import}</button>
      <button class="btn small quiet" class:on={showSettings} aria-expanded={showSettings} onclick={() => (showSettings = !showSettings)}>{t.settings}</button>
    </div>
  </div>

  {#if showSettings && settings}
    <section class="panel settings">
      <div class="setting">
        <div class="what"><b>{t.when}</b><span>{t.whenHint}</span></div>
        <div class="how choices" role="radiogroup" aria-label={t.when}>
          {#each [["after", t.whenAfter], ["live", t.whenLive], ["manual", t.whenManual]] as [id, label] (id)}
            <button
              class="choice"
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
        <div class="how choices" role="radiogroup" aria-label={t.summaryBy}>
          <button class="choice" class:on={settings.summary_by === "local"} role="radio" aria-checked={settings.summary_by === "local"} onclick={() => saveSettings({ summary_by: "local" })}>{t.thisComputer}</button>
          <button class="choice" class:on={settings.summary_by === "brain"} role="radio" aria-checked={settings.summary_by === "brain"} disabled={!summarizers?.brain} title={summarizers?.brain ?? t.noBrain} onclick={() => saveSettings({ summary_by: "brain" })}>{summarizers?.brain ?? t.brain}</button>
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
            <span class="bar"><span style:width="{Math.round(Math.sqrt(store.levels[track.name] ?? 0) * 100)}%"></span></span>
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
          <button class="source" class:on={played === "system"} role="radio" aria-checked={played === "system"} onclick={() => (played = "system")}>
            <span class="glyph"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h16v11H4zM9 20h6M12 16v4" /></svg></span>
            <span class="label">{t.everything}</span>
          </button>
          {#each sources?.apps ?? [] as app (app.id)}
            <button class="source" class:on={played === app.id} role="radio" aria-checked={played === app.id} title={app.name} onclick={() => (played = app.id)}>
              {#if app.icon}
                <img src="data:image/png;base64,{app.icon}" alt="" />
              {:else}
                <span class="glyph letter">{app.name.slice(0, 1)}</span>
              {/if}
              {#if app.playing}<span class="playing" title={t.playingNow}>♪</span>{/if}
              <span class="label">{app.name}</span>
            </button>
          {/each}
          <button class="source" class:on={played === "none"} role="radio" aria-checked={played === "none"} onclick={() => (played = "none")}>
            <span class="glyph"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17ZM6 6l12 12" /></svg></span>
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
            {#if m.state !== "recording"}
              <span class="end">
                {#if m.id in store.transcribing}
                  <span class="faint small">{t.transcribing(percent(store.transcribing[m.id]))}</span>
                {:else if m.id in store.summarizing}
                  <span class="faint small">{t.summarizing(percent(store.summarizing[m.id]))}</span>
                {:else if m.transcribed_at}
                  <button class="btn small" class:on={open === m.id} aria-expanded={open === m.id} onclick={() => toggleTranscript(m)}>{open === m.id ? t.hideTranscript : t.transcript}</button>
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
                <audio controls src={audio[key]} bind:this={players[key]}></audio>
              {:else if track.file}
                <button class="btn small" onclick={() => listen(m, track)}>{t.listen}</button>
              {:else}
                <span class="faint small">{t.noFile}</span>
              {/if}
            </div>
          {/each}
          {#if open === m.id}
            {@const transcript = transcripts[m.id]}
            {@const summary = summaries[m.id]}
            <div class="summary">
              {#if summary}
                <header class="summary-head">
                  <b>{summary.title}</b>
                  <span class="end">
                    <button class="btn small quiet" onclick={() => copySummary(m, summary)}>{copied === m.id ? t.copied : t.copy}</button>
                    <button class="btn small quiet" disabled={m.id in store.summarizing || !canSummarize} onclick={() => summarize(m)}>{t.summarizeAgain}</button>
                  </span>
                </header>
                <p class="overview">{summary.overview}</p>
                {#each [{ title: t.points, items: summary.points }, { title: t.decisions, items: summary.decisions }] as part (part.title)}
                  {#if part.items.length}
                    <h3>{part.title}</h3>
                    <ul>{#each part.items as item, i (i)}<li>{item}</li>{/each}</ul>
                  {/if}
                {/each}
                {#if summary.actions.length}
                  <h3>{t.actions}</h3>
                  <ul class="todo">
                    {#each summary.actions as a, i (i)}
                      <li>{a.text}{#if a.owner || a.due}<span class="faint"> · {[a.owner, a.due].filter(Boolean).join(", ")}</span>{/if}</li>
                    {/each}
                  </ul>
                {/if}
                {#if summary.questions.length}
                  <h3>{t.questions}</h3>
                  <ul>{#each summary.questions as item, i (i)}<li>{item}</li>{/each}</ul>
                {/if}
                <p class="faint small by">{t.summaryBy}: {summary.by}</p>
              {:else if summary === null && !(m.id in store.summarizing)}
                <button class="btn small" disabled={!canSummarize} title={canSummarize ? undefined : t.needSummarizer} onclick={() => summarize(m)}>{t.summarize}</button>
              {/if}
            </div>
            <div class="transcript">
              <div class="exports">
                <span class="faint small">{t.exportAs}</span>
                <button class="btn small quiet" onclick={() => exportAs(m, "markdown")}>Markdown</button>
                <button class="btn small quiet" onclick={() => exportAs(m, "srt")}>SRT</button>
                <button class="btn small quiet" onclick={() => exportAs(m, "text")}>{t.plainText}</button>
                <button class="btn small quiet again" disabled={m.id in store.transcribing || !model} onclick={() => transcribe(m)}>{t.again}</button>
                {#if exported}<span class="faint small saved">{t.exported(exported)}</span>{/if}
              </div>
              {#if transcript?.lines.length}
                {#if transcript.speakers.length > 1}
                  <div class="people">
                    {#each transcript.speakers as s (s.key)}
                      {#if naming === `${m.id}/${s.key}`}
                        <!-- svelte-ignore a11y_autofocus -->
                        <input
                          class="person-name"
                          type="text"
                          bind:value={nameDraft}
                          autofocus
                          placeholder={label({ ...s, name: null })}
                          aria-label={t.nameSpeaker}
                          onblur={() => saveName(m, s)}
                          onkeydown={(e) => (e.key === "Enter" ? saveName(m, s) : e.key === "Escape" ? (naming = null) : null)}
                        />
                      {:else}
                        <button class="person" style:--tint={tint(s)} title={t.nameSpeaker} onclick={() => startNaming(m, s)}>{label(s)}</button>
                      {/if}
                    {/each}
                    <span class="faint small">{t.nameHint}</span>
                  </div>
                {/if}
                {#each transcript.lines as line, i (i)}
                  {@const s = speakerOf(transcript, line)}
                  {@const same = i > 0 && speakerOf(transcript, transcript.lines[i - 1]) === s}
                  <div class="line">
                    <button class="at mono" title={t.playFrom} onclick={() => seek(m, line)}>{clock(line.start_ms / 1000)}</button>
                    <span class="who" style:color={tint(s)}>{same ? "" : label(s)}</span>
                    <p>{line.text}</p>
                  </div>
                {/each}
              {:else if transcript}
                <p class="faint small">{t.noSpeech}</p>
              {/if}
            </div>
          {/if}
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
  .meetings {
    max-width: 820px;
  }
  .small {
    font-size: 12.5px;
  }
  .head-actions {
    display: flex;
    gap: 8px;
  }
  .btn.on {
    background: var(--ink-3);
  }
  .settings {
    margin-bottom: 14px;
  }
  .choices {
    gap: 4px;
  }
  .choice {
    height: 30px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--rim);
    background: transparent;
    color: var(--haze);
    font: 500 12.5px/1 var(--sans);
    cursor: pointer;
  }
  .choice.on {
    background: var(--ink-3);
    border-color: var(--rim-2);
    color: var(--pearl);
  }
  .choice:disabled {
    opacity: 0.45;
    cursor: default;
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
    font: 600 18px/1.2 var(--display);
    color: var(--pearl);
  }
  .summary {
    margin-top: 12px;
    padding: 14px 16px;
    border-radius: var(--r);
    background: var(--ink-2);
  }
  .summary:empty {
    display: none;
  }
  .summary-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .summary-head b {
    font: 600 15px/1.3 var(--sans);
  }
  .overview {
    margin-top: 6px;
    color: var(--pearl);
    font-size: 14px;
    line-height: 1.5;
    user-select: text;
  }
  .summary h3 {
    margin: 12px 0 4px;
    font: 600 11px/1 var(--sans);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--dusk);
  }
  .summary ul {
    padding-left: 18px;
    color: var(--pearl);
    font-size: 13.5px;
    line-height: 1.5;
    user-select: text;
  }
  .todo li::marker {
    content: "☐  ";
  }
  .by {
    margin-top: 10px;
  }
  .transcript {
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px dashed var(--rim);
  }
  .exports {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }
  .saved {
    user-select: text;
  }
  .again {
    margin-left: auto;
  }
  .line {
    display: grid;
    grid-template-columns: 58px 96px minmax(0, 1fr);
    gap: 10px;
    align-items: baseline;
    padding: 5px 0;
  }
  .at {
    all: unset;
    cursor: pointer;
    color: var(--dusk);
  }
  .at:hover {
    color: var(--pearl);
  }
  .who {
    font-size: 12px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .people {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 10px;
  }
  .person {
    height: 26px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--tint) 45%, transparent);
    background: color-mix(in srgb, var(--tint) 12%, transparent);
    color: var(--tint);
    font: 600 12px/1 var(--sans);
    cursor: text;
  }
  .person-name {
    height: 26px;
    width: 140px;
    font-size: 12px;
  }
  .line p {
    color: var(--pearl);
    font-size: 14px;
    line-height: 1.5;
    user-select: text;
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
    font-size: 12.5px;
  }
  .bar {
    height: 6px;
    border-radius: 999px;
    background: var(--ink-3);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--pearl-mid);
    transition: width 200ms linear;
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
    display: grid;
    justify-items: center;
    gap: 6px;
    width: 88px;
    padding: 10px 6px 8px;
    border-radius: var(--r);
    border: 1px solid transparent;
    background: transparent;
    color: var(--haze);
    cursor: pointer;
    font: 500 11.5px/1.2 var(--sans);
  }
  .source:hover {
    background: var(--ink-2);
  }
  .source.on {
    background: var(--ink-3);
    border-color: var(--rim-2);
    color: var(--pearl);
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
  .glyph svg {
    width: 24px;
    height: 24px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .letter {
    font: 600 20px/1 var(--display);
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
    font-size: 11px;
  }
  .title {
    flex: 1;
    max-width: 360px;
  }
  .note {
    font-size: 12px;
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
