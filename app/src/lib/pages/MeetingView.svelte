<!--
  A meeting, in a tab of its own (docs/meetings.md): its tracks to listen
  to, and its summary or its transcript, one at a time. The summary (what
  was said, decided, left to do and left open) can be copied as Markdown or
  written again; the transcript names who speaks (each nameable, a colour
  each), jumps to a line's moment in its track, and exports as Markdown,
  SRT or plain text. Recording and the list are on the Meetings page.
-->
<script lang="ts">
  import { tick, untrack } from "svelte";
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Meeting, MeetingSettings, MeetingTrack, Speaker, Summarizers, Summary, Transcript, TranscriptLine } from "../types";
  import { clock } from "../ui/when";

  let { id }: { id: string } = $props();
  const t = $derived(i18n.t.meetings);
  const th = $derived(i18n.t.home);
  const m = $derived(store.meetings.find((x) => x.id === id) ?? null);

  let view = $state<"summary" | "transcript">("summary");
  let transcript = $state<Transcript | null | undefined>(undefined);
  let summary = $state<Summary | null | undefined>(undefined);
  let settings = $state<MeetingSettings | null>(null);
  let summarizers = $state<Summarizers | null>(null);
  let model = $state<boolean | null>(null);
  let error = $state("");
  let exported = $state("");
  let copied = $state(false);
  /** Blob URLs of the tracks loaded for listening, by name. */
  let audio = $state<Record<string, string>>({});
  const players: Record<string, HTMLAudioElement> = {};

  $effect(() => {
    const want = id;
    untrack(() => {
      transcript = undefined;
      summary = undefined;
      exported = "";
    });
    call<Transcript | null>("meeting_transcript", { id: want }).then((x) => want === id && (transcript = x), () => (transcript = null));
    call<Summary | null>("meeting_summary", { id: want }).then(
      (x) => {
        if (want !== id) return;
        summary = x;
        // No summary yet: the transcript is what there is to read.
        if (!x) view = "transcript";
      },
      () => (summary = null),
    );
    return () => untrack(() => Object.values(audio).forEach((url) => URL.revokeObjectURL(url)));
  });
  $effect(() => {
    call<MeetingSettings>("meeting_settings").then((x) => (settings = x), () => {});
    call<Summarizers>("meeting_summarizers").then((x) => (summarizers = x), () => {});
    call<boolean>("meeting_model").then((x) => (model = x), () => {});
  });

  // A transcript made again, or a summary written, shows at once.
  $effect(() => {
    const done = store.recorded;
    if (done?.id === id && done.transcribed_at) untrack(() => call<Transcript | null>("meeting_transcript", { id }).then((x) => (transcript = x), () => {}));
  });
  $effect(() => {
    const done = store.summarized;
    if (done?.id === id) untrack(() => ((summary = done.summary), (view = "summary")));
  });

  /** Whether the settings' summarizer is there. */
  const canSummarize = $derived(
    !!settings &&
      !!summarizers &&
      (settings.summary_by === "local" ? summarizers.local : settings.summary_by === "brain" ? !!summarizers.brain : summarizers.models.some((x) => x.id === settings!.summary_by)),
  );

  async function summarize() {
    error = "";
    store.summarizing = { ...store.summarizing, [id]: 0 };
    try {
      await call("meeting_summarize", { id });
    } catch (e) {
      error = why(e);
    }
  }

  async function transcribe() {
    error = "";
    store.transcribing = { ...store.transcribing, [id]: 0 };
    try {
      await call("meeting_transcribe", { id });
    } catch (e) {
      error = why(e);
    }
  }

  /** The summary as Markdown, on the clipboard. */
  async function copySummary(s: Summary) {
    const list = (title: string, items: string[]) => (items.length ? `\n## ${title}\n\n${items.map((x) => `- ${x}`).join("\n")}\n` : "");
    const actions = s.actions.map((a) => {
      const who = [a.owner, a.due].map((x) => x.trim()).filter(Boolean).join(", ");
      return who ? `${a.text} (${who})` : a.text;
    });
    const text = `# ${s.title}\n\n${s.overview}\n${list(t.points, s.points)}${list(t.decisions, s.decisions)}${list(t.actions, actions)}${list(t.questions, s.questions)}`;
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch (e) {
      error = why(e);
    }
  }

  async function exportAs(format: "markdown" | "srt" | "text") {
    error = "";
    try {
      exported = await call<string>("meeting_export", { id, format, you: t.you, person: t.personWord });
    } catch (e) {
      error = why(e);
    }
  }

  async function listen(track: MeetingTrack) {
    if (audio[track.name]) return;
    error = "";
    try {
      const raw = await call<ArrayBuffer | Uint8Array | number[]>("meeting_audio", { id, track: track.name });
      const bytes = raw instanceof ArrayBuffer ? raw : ArrayBuffer.isView(raw) ? raw : Uint8Array.from(raw);
      const type = track.file?.endsWith(".caf") ? "audio/x-caf" : "audio/mp4";
      audio = { ...audio, [track.name]: URL.createObjectURL(new Blob([bytes as BlobPart], { type })) };
    } catch (e) {
      error = why(e);
    }
  }

  /** Play the line's track from where it starts. */
  async function seek(meeting: Meeting, line: TranscriptLine) {
    const track = meeting.tracks.find((x) => x.name === line.track);
    if (!track) return;
    await listen(track);
    await tick();
    const player = players[track.name];
    if (!player) return;
    player.currentTime = line.start_ms / 1000;
    player.play().catch(() => {});
  }

  // Who speaks: a name each, a colour each, the same in every line.
  let naming = $state<string | null>(null);
  let nameDraft = $state("");
  const speakerOf = (tr: Transcript, line: TranscriptLine): Speaker | undefined => tr.speakers.find((s) => s.key === `${line.track}/${line.speaker ?? "-"}`);
  const label = (s: Speaker | undefined) => s?.name ?? (!s ? "?" : s.number === 0 ? t.you : t.person(s.number));
  const TINTS = ["var(--think)", "#c58cff", "#5fd4c8", "var(--wait)", "#ff7eb6", "var(--done)", "#f5d061", "#8aa4ff"];
  const tint = (s: Speaker | undefined) => (s ? TINTS[s.number % TINTS.length] : "var(--dusk)");

  async function saveName(s: Speaker) {
    if (naming !== s.key) return;
    naming = null;
    if ((s.name ?? "") === nameDraft.trim()) return;
    try {
      transcript = await call<Transcript>("rename_speaker", { id, key: s.key, name: nameDraft });
    } catch (e) {
      error = why(e);
    }
  }

  const percent = (f: number) => Math.round(f * 100);
  const name = (x: Meeting) => x.title ?? t.untitled(`${i18n.date(x.started_at)} · ${i18n.time(x.started_at)}`);
</script>

<div class="page meeting">
  {#if m}
    <div class="page-head">
      <div>
        <h1>{name(m)}</h1>
        <p class="lede">
          {i18n.date(m.started_at, { weekday: "long", day: "numeric", month: "long" })} · {i18n.time(m.started_at)} · {clock(m.duration_ms / 1000)}
          {#if m.state === "recording"} · {t.recordingNow}{/if}
        </p>
      </div>
      <div class="actions">
        {#if summary && view === "summary"}
          <button class="btn" onclick={() => copySummary(summary!)}>{copied ? t.copied : t.copy}</button>
        {/if}
      </div>
    </div>

    {#if m.tracks.some((x) => x.file)}
      <div class="tracks well">
        {#each m.tracks as track (track.name)}
          <div class="track">
            <span class="dim small">{track.kind === "mic" ? t.you : t.others} · {track.source}</span>
            {#if audio[track.name]}
              <audio controls src={audio[track.name]} bind:this={players[track.name]}></audio>
            {:else if track.file}
              <button class="btn small" onclick={() => listen(track)}>{t.listen}</button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}

    <div class="seg" role="tablist">
      <button role="tab" class:on={view === "summary"} aria-selected={view === "summary"} onclick={() => (view = "summary")}>{th.summaryTab}</button>
      <button role="tab" class:on={view === "transcript"} aria-selected={view === "transcript"} onclick={() => (view = "transcript")}>{th.transcriptTab}</button>
    </div>

    {#if error}<p class="error-text">{error}</p>{/if}
    {#if store.transcribeErrors[id]}<p class="error-text">{t.failed(store.transcribeErrors[id])}</p>{/if}
    {#if store.summaryErrors[id]}<p class="error-text">{t.summaryFailed(store.summaryErrors[id])}</p>{/if}

    {#if view === "summary"}
      <section class="summary">
        {#if id in store.summarizing}
          <p class="faint">{t.summarizing(percent(store.summarizing[id]))}</p>
        {:else if summary}
          <h2>{summary.title}</h2>
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
          <p class="faint small by">
            {t.summaryBy}: {summary.by}
            <button class="btn link small" disabled={!canSummarize} onclick={summarize}>{t.summarizeAgain}</button>
          </p>
        {:else if summary === null}
          <div class="empty">
            <p>{m.transcribed_at ? t.summarize : t.needModel}</p>
            {#if m.transcribed_at}
              <button class="btn primary" disabled={!canSummarize} title={canSummarize ? undefined : t.needSummarizer} onclick={summarize}>{t.summarize}</button>
            {/if}
          </div>
        {/if}
      </section>
    {:else}
      <section class="transcript">
        {#if id in store.transcribing}
          <p class="faint">{t.transcribing(percent(store.transcribing[id]))}</p>
        {:else if !m.transcribed_at}
          <div class="empty">
            <button class="btn primary" disabled={!model} title={model ? undefined : t.needModel} onclick={transcribe}>{t.transcribe}</button>
          </div>
        {:else}
          <div class="exports">
            <span class="faint small">{t.exportAs}</span>
            <button class="btn small quiet" onclick={() => exportAs("markdown")}>Markdown</button>
            <button class="btn small quiet" onclick={() => exportAs("srt")}>SRT</button>
            <button class="btn small quiet" onclick={() => exportAs("text")}>{t.plainText}</button>
            <button class="btn small quiet again" disabled={!model} onclick={transcribe}>{t.again}</button>
            {#if exported}<span class="faint small saved">{t.exported(exported)}</span>{/if}
          </div>
          {#if transcript?.lines.length}
            {#if transcript.speakers.length > 1}
              <div class="people">
                {#each transcript.speakers as s (s.key)}
                  {#if naming === s.key}
                    <input
                      class="compact person-name"
                      type="text"
                      bind:value={nameDraft}
                      placeholder={label({ ...s, name: null })}
                      aria-label={t.nameSpeaker}
                      {@attach (el: HTMLInputElement) => el.focus()}
                      onblur={() => saveName(s)}
                      onkeydown={(e) => (e.key === "Enter" ? saveName(s) : e.key === "Escape" ? (naming = null) : null)}
                    />
                  {:else}
                    <button class="chip small person" style:--tint={tint(s)} title={t.nameSpeaker} onclick={() => ((naming = s.key), (nameDraft = s.name ?? ""))}>{label(s)}</button>
                  {/if}
                {/each}
                <span class="faint small">{t.nameHint}</span>
              </div>
            {/if}
            {#each transcript.lines as line, i (i)}
              {@const s = speakerOf(transcript, line)}
              {@const same = i > 0 && speakerOf(transcript, transcript.lines[i - 1]) === s}
              <div class="line">
                <button class="btn link at mono" title={t.playFrom} onclick={() => seek(m!, line)}>{clock(line.start_ms / 1000)}</button>
                <span class="who" style:color={tint(s)}>{same ? "" : label(s)}</span>
                <p>{line.text}</p>
              </div>
            {/each}
          {:else if transcript}
            <p class="faint">{t.noSpeech}</p>
          {/if}
        {/if}
      </section>
    {/if}
  {/if}
</div>

<style>
  .small {
    font-size: var(--fs-meta);
  }
  .tracks {
    display: grid;
    gap: 8px;
    margin-bottom: 20px;
    padding: 10px 14px;
  }
  .track {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
    min-height: 32px;
  }
  audio {
    height: 32px;
    width: 300px;
  }
  .seg {
    margin-bottom: 22px;
  }
  .summary h2 {
    font: 600 var(--fs-head)/1.3 var(--display);
  }
  .overview {
    margin-top: 8px;
    color: var(--pearl);
    font-size: var(--fs-body);
    line-height: 1.55;
    user-select: text;
  }
  .summary h3 {
    margin: 18px 0 6px;
    font: 600 var(--fs-body)/1.3 var(--sans);
    color: var(--pearl);
  }
  .summary ul {
    padding-left: 18px;
    color: var(--pearl);
    font-size: var(--fs-body);
    line-height: 1.55;
    user-select: text;
  }
  .todo li::marker {
    content: "☐  ";
  }
  .by {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 18px;
  }
  .empty {
    display: grid;
    justify-items: center;
    gap: 12px;
    padding: 30px 0;
  }
  .exports {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 12px;
  }
  .saved {
    user-select: text;
  }
  .again {
    margin-left: auto;
  }
  .people {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 12px;
  }
  .person {
    border-color: color-mix(in srgb, var(--tint) 45%, transparent);
    background: color-mix(in srgb, var(--tint) 12%, transparent);
    color: var(--tint);
    font-weight: 600;
    cursor: text;
  }
  .person-name {
    width: 140px;
  }
  .line {
    display: grid;
    grid-template-columns: 58px 96px minmax(0, 1fr);
    gap: 10px;
    align-items: baseline;
    padding: 5px 0;
  }
  .at {
    color: var(--dusk);
    text-decoration: none;
  }
  .at:hover {
    color: var(--pearl);
  }
  .who {
    font-size: var(--fs-meta);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .line p {
    color: var(--pearl);
    font-size: var(--fs-body);
    line-height: 1.5;
    user-select: text;
  }
</style>
