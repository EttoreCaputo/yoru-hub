<!--
  A meeting, in a tab of its own (docs/meetings.md): its tracks to listen
  to, and its summary or its transcript, one at a time. The summary (the
  gist, sections laid out by its style, what was decided, left to do and
  left open) can be copied as Markdown or written again in another style;
  a to-do shows its day and, for a message or an email, a draft to copy; the transcript names who speaks (each nameable, a colour
  each), jumps to a line's moment in its track, and exports as Markdown,
  SRT or plain text. Recording and the list are on the Meetings page.
-->
<script lang="ts">
  import { tick, untrack } from "svelte";
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ActionItem, Meeting, MeetingSettings, MeetingTrack, Speaker, Summarizers, Summary, Transcript, TranscriptLine } from "../types";
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
  /** The style to summarize again in; the summary's own until changed. */
  let style = $state("");
  /** The to-do whose draft is open, and the one whose draft was just copied. */
  let drafting = $state<number | null>(null);
  let draftCopied = $state<number | null>(null);
  /** Blob URLs of the tracks loaded for listening, by name. */
  let audio = $state<Record<string, string>>({});
  const players: Record<string, HTMLAudioElement> = {};

  $effect(() => {
    const want = id;
    untrack(() => {
      transcript = undefined;
      summary = undefined;
      exported = "";
      style = "";
      drafting = null;
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

  const STYLES = ["auto", "meeting", "call", "interview", "lecture", "note"] as const;
  /** The style shown in the picker: chosen here, else the summary's, else the settings'. */
  const picked = $derived(style || summary?.style || settings?.summary_style || "auto");

  async function summarize() {
    error = "";
    store.summarizing = { ...store.summarizing, [id]: 0 };
    try {
      await call("meeting_summarize", { id, style: picked });
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

  /** When a to-do is due: its day, else what was said. */
  const dueOf = (a: ActionItem) => {
    const [y, mo, d] = (a.date ?? "").split("-").map(Number);
    return y && mo && d ? i18n.date(new Date(y, mo - 1, d).getTime(), { weekday: "short", day: "numeric", month: "short" }) : a.due;
  };
  const metaOf = (a: ActionItem) => [a.owner, dueOf(a)].map((x) => x?.trim()).filter(Boolean).join(" · ");
  const kindOf = (a: ActionItem) => (a.kind in t.kinds ? a.kind : "task") as keyof typeof t.kinds;

  /** A line's **bold** stretches apart from the rest, to show without HTML. */
  const bolds = (text: string) => text.split("**").map((part, i) => ({ part, bold: i % 2 === 1 }));

  /** The summary as Markdown, on the clipboard. */
  async function copySummary(s: Summary) {
    const list = (title: string, items: string[]) => (items.length ? `\n## ${title}\n\n${items.map((x) => `- ${x}`).join("\n")}\n` : "");
    const actions = s.actions.map((a) => {
      const meta = metaOf(a);
      return `[ ] ${a.text}${meta ? ` (${meta})` : ""}${a.draft ? `\n  > ${a.draft.replaceAll("\n", "\n  > ")}` : ""}`;
    });
    const body = (s.sections ?? []).map((x) => list(x.title, x.items)).join("") + list(t.points, s.points ?? []);
    const text = `# ${s.title}\n\n${s.overview}\n${body}${list(t.decisions, s.decisions)}${list(t.actions, actions)}${list(t.questions, s.questions)}`;
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch (e) {
      error = why(e);
    }
  }

  async function copyDraft(i: number, draft: string) {
    try {
      await navigator.clipboard.writeText(draft);
      draftCopied = i;
      setTimeout(() => draftCopied === i && (draftCopied = null), 1600);
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
          {#each [...(summary.sections ?? []), { title: t.points, items: summary.points ?? [] }, { title: t.decisions, items: summary.decisions }] as part, n (n)}
            {#if part.items.length}
              <h3>{part.title}</h3>
              <ul>
                {#each part.items as item, i (i)}
                  <li>{#each bolds(item) as x, k (k)}{#if x.bold}<b>{x.part}</b>{:else}{x.part}{/if}{/each}</li>
                {/each}
              </ul>
            {/if}
          {/each}
          {#if summary.actions.length}
            <h3>{t.actions}</h3>
            <ul class="todo">
              {#each summary.actions as a, i (i)}
                <li>
                  <div class="do">
                    <span class="kind {kindOf(a)}">{t.kinds[kindOf(a)]}</span>
                    <span class="what">{a.text}{#if metaOf(a)}<span class="faint meta">· {metaOf(a)}</span>{/if}</span>
                    {#if a.draft}
                      <button class="btn link small" aria-expanded={drafting === i} onclick={() => (drafting = drafting === i ? null : i)}>{t.showDraft}</button>
                    {/if}
                  </div>
                  {#if a.draft && drafting === i}
                    <div class="draft well">
                      <p>{a.draft}</p>
                      <button class="btn small" onclick={() => copyDraft(i, a.draft)}>{draftCopied === i ? t.copied : t.copyDraft}</button>
                    </div>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if summary.questions.length}
            <h3>{t.questions}</h3>
            <ul>{#each summary.questions as item, i (i)}<li>{item}</li>{/each}</ul>
          {/if}
          <div class="faint small by">
            <span>{t.summaryBy}: {summary.by}{#if summary.style in t.styles} · {t.styles[summary.style as keyof typeof t.styles]}{/if}</span>
            <span class="again-with">
              <select class="compact" value={picked} onchange={(e) => (style = e.currentTarget.value)} aria-label={t.style}>
                {#each STYLES as s (s)}<option value={s}>{t.styles[s]}</option>{/each}
              </select>
              <button class="btn small" disabled={!canSummarize} onclick={summarize}>{t.summarizeAgain}</button>
            </span>
          </div>
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
  .summary b {
    font-weight: 600;
  }
  .todo {
    list-style: none;
    padding-left: 0;
    display: grid;
    gap: 8px;
  }
  .do {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .do .what {
    flex: 1;
    min-width: 0;
  }
  .meta {
    margin-left: 6px;
  }
  .kind {
    flex-shrink: 0;
    min-width: 76px;
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid var(--rim);
    color: var(--haze);
    font-size: var(--fs-small);
    text-align: center;
  }
  .kind.reminder {
    color: var(--wait);
    border-color: color-mix(in srgb, var(--wait) 40%, transparent);
  }
  .kind.message {
    color: var(--done);
    border-color: color-mix(in srgb, var(--done) 40%, transparent);
  }
  .kind.email {
    color: var(--think);
    border-color: color-mix(in srgb, var(--think) 40%, transparent);
  }
  .draft {
    display: grid;
    justify-items: start;
    gap: 8px;
    margin: 8px 0 4px 86px;
    padding: 10px 12px;
  }
  .draft p {
    white-space: pre-wrap;
    user-select: text;
    color: var(--pearl);
  }
  .by {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 22px;
  }
  .again-with {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
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
