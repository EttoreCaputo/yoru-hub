<!--
  Settings > Meetings (docs/meetings.md): what happens to a meeting, in the
  order it happens: noticing a call, the transcript made on this Mac, and
  the summary written from it. Each step says where it runs and what it
  needs downloaded.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { MeetingSettings, Summarizers } from "../types";
  import ModelPicker, { type Choice } from "./ModelPicker.svelte";
  import Toggle from "./Toggle.svelte";

  const t = $derived(i18n.t.meetings);
  let settings = $state<MeetingSettings | null>(null);
  /** Whether Parakeet is downloaded; null until known. */
  let model = $state<boolean | null>(null);
  let summarizers = $state<Summarizers | null>(null);
  let fetchingLlm = $state(false);
  let error = $state("");

  $effect(() => {
    call<MeetingSettings>("meeting_settings").then((x) => (settings = x), (e) => (error = why(e)));
    call<boolean>("meeting_model").then((x) => (model = x), () => {});
    call<Summarizers>("meeting_summarizers").then((x) => (summarizers = x), () => {});
  });

  async function save(change: Partial<MeetingSettings>) {
    if (!settings) return;
    error = "";
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

  /** The services with a key: every model of theirs can summarize. */
  const SERVICES = [
    ["anthropic", "Anthropic"],
    ["openai", "OpenAI"],
    ["google", "Google"],
    ["groq", "Groq"],
  ] as const;
  const keyed = $derived.by(() => {
    const i = store.info;
    const has: Record<string, unknown> = { anthropic: i?.key, openai: i?.openai_key, google: i?.google_key, groq: i?.groq_key };
    const out: { id: string; name: string }[] = SERVICES.filter(([id]) => has[id]).map(([id, name]) => ({ id, name }));
    if (summarizers?.models.some((m) => m.id.startsWith("ollama:"))) out.push({ id: "ollama", name: "Ollama" });
    return out;
  });
  const described = (m: Summarizers["models"][number]): Choice => ({ value: m.id, label: m.note.split(":")[0] || m.model, note: m.note.split(":").slice(1).join(":").trim(), group: i18n.t.picker.suggested });
  /** A transcript: Parakeet here, or a Gemini model that hears a whole recording (not a Live one). */
  const transcribers = $derived<Choice[]>([{ value: "local", label: t.parakeet, note: t.simpleLocal }, ...(summarizers?.models ?? []).filter((m) => m.id.startsWith("google:")).map(described)]);
  /** A summary: this computer, the brain, or the catalog's described models. */
  const summarizerChoices = $derived<Choice[]>([
    { value: "local", label: t.thisComputer, note: t.simpleLocalSummary },
    ...(summarizers?.brain ? [{ value: "brain", label: `${t.brain} (${summarizers.brain})`, note: t.simpleBrainSummary }] : []),
    ...(summarizers?.models ?? []).map(described),
  ]);
  const parakeet = $derived(settings?.transcribe_by === "local");
  const percent = (f: number) => Math.round(f * 100);
  const LANGUAGES = ["", "it", "en", "es", "fr", "de", "pt"];
  const WHEN = $derived([
    ["after", t.whenAfter],
    ["manual", t.whenManual],
    ["live", t.whenLive],
  ] as const);
</script>

{#if settings}
  <section class="panel">
    <div class="setting">
      <div class="what"><b>{t.simpleTranscript}</b><span>{t.transcribeByHint}</span></div>
      <div class="how">
        <ModelPicker value={settings.transcribe_by} choices={transcribers} providers={store.info?.google_key ? [{ id: "google", name: "Google" }] : []} need="stt" accept={(m) => !m.model.includes("-live")} label={t.simpleTranscript} onchange={(v) => save({ transcribe_by: v })} />
      </div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.simpleSummary}</b><span>{t.summaryByHint}</span></div>
      <div class="how">
        <ModelPicker value={settings.summary_by} choices={summarizerChoices} providers={keyed} need="chat" label={t.simpleSummary} onchange={(v) => save({ summary_by: v })} />
      </div>
    </div>
    {#if settings.transcribe_by.startsWith("google:") && !store.info?.google_key}<div class="setting"><p class="error-text">{t.noGoogleKey}</p></div>{/if}
    {#if summarizers && ((settings.summary_by === "local" && !summarizers.local) || (settings.summary_by === "brain" && !summarizers.brain))}<div class="setting"><p class="error-text">{t.needSummarizer}</p></div>{/if}
  </section>
  <h2 class="section-title">{t.stepNotice}</h2>
  <section class="panel">
    <div class="setting">
      <div class="what"><b>{t.detect}</b><span>{t.detectHint}</span></div>
      <div class="how"><Toggle checked={settings.detect} label={t.detect} onchange={(on) => save({ detect: on })} /></div>
    </div>
  </section>

  <h2 class="section-title">{t.stepTranscript}</h2>
  <p class="dim note">{t.stepTranscriptNote}</p>
  <section class="panel">
    {#if parakeet}
      <div class="setting">
        <div class="what"><b>{t.model}</b><span>{t.modelHint}</span></div>
        <div class="how">
          {#if store.modelDownload !== null}
            <span class="dim small">{t.downloading(percent(store.modelDownload))}</span>
          {:else if model}
            <span class="state"><span class="dot done"></span>{t.modelReady}</span>
          {:else if model === false}
            <button class="btn small primary" onclick={downloadModel}>{t.download}</button>
          {/if}
        </div>
      </div>
    {/if}
    <div class="setting stack">
      <div class="what"><b>{t.when}</b><span>{t.whenHint}</span></div>
      <div class="how chips" role="radiogroup" aria-label={t.when}>
        {#each WHEN as [id, label] (id)}
          <button
            class="chip"
            class:on={settings.transcribe === id}
            role="radio"
            aria-checked={settings.transcribe === id}
            disabled={id === "live"}
            title={id === "live" ? t.liveSoon : undefined}
            onclick={() => save({ transcribe: id })}>{label}</button
          >
        {/each}
      </div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.language}</b><span>{t.languageHint}</span></div>
      <div class="how">
        <select value={settings.language} onchange={(e) => save({ language: e.currentTarget.value })} aria-label={t.language}>
          {#each LANGUAGES as code (code)}<option value={code}>{code ? t.languages[code as keyof typeof t.languages] : t.languageAuto}</option>{/each}
        </select>
      </div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.speakers}</b><span>{t.speakersHint}</span></div>
      <div class="how"><Toggle checked={settings.speakers} label={t.speakers} onchange={(on) => save({ speakers: on })} /></div>
    </div>
  </section>

  <h2 class="section-title">{t.stepSummary}</h2>
  <p class="dim note">{t.stepSummaryNote}</p>
  <section class="panel">
    {#if settings.summary_by === "local" && summarizers && !summarizers.local}
      <div class="setting">
        <div class="what"><b>{t.llm}</b><span>{t.llmHint}</span></div>
        <div class="how"><button class="btn small primary" disabled={fetchingLlm} onclick={fetchLlm}>{fetchingLlm ? t.fetching : t.download}</button></div>
      </div>
    {/if}
    <div class="setting">
      <div class="what"><b>{t.style}</b><span>{t.styleHint}</span></div>
      <div class="how">
        <select value={settings.summary_style || "auto"} onchange={(e) => save({ summary_style: e.currentTarget.value })} aria-label={t.style}>
          {#each Object.entries(t.styles) as [id, label] (id)}<option value={id}>{label}</option>{/each}
        </select>
      </div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.autoSummary}</b><span>{t.autoSummaryHint}</span></div>
      <div class="how"><Toggle checked={settings.auto_summary} label={t.autoSummary} onchange={(on) => save({ auto_summary: on })} /></div>
    </div>
  </section>
{/if}
{#if error}<p class="error-text note">{error}</p>{/if}

<style>
  .note {
    font-size: var(--fs-meta);
    margin: -4px 0 10px;
  }
  .small {
    font-size: var(--fs-meta);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--haze);
    font-size: var(--fs-meta);
    white-space: nowrap;
  }
  select {
    max-width: 340px;
  }
  /* Three choices don't fit beside a hint: under it. */
  .stack {
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
  }
  .stack .how {
    flex-wrap: wrap;
  }
  .section-title + .panel,
  .note + .panel {
    margin-bottom: 22px;
  }
</style>
