<!--
  Settings > Meetings (docs/meetings.md): what happens to a meeting, in the
  order it happens: noticing a call, the transcript made on this Mac, and
  the summary written from it. Who transcribes and who summarizes, and
  Parakeet's download, are in Settings > Intelligence with every model.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { serviceOf } from "../meeting-readiness";
  import type { MeetingSettings, Summarizers } from "../types";
  import Icon from "./Icon.svelte";
  import Toggle from "./Toggle.svelte";

  let { onmodels }: { onmodels?: (e: Event) => void } = $props();

  const t = $derived(i18n.t.meetings);
  let settings = $state<MeetingSettings | null>(null);
  let summarizers = $state<Summarizers | null>(null);
  let error = $state("");

  $effect(() => {
    call<MeetingSettings>("meeting_settings").then((x) => (settings = x), (e) => (error = why(e)));
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

  const simple = $derived(settings?.detail === "simple");
  /** The transcriber chosen, as the app offers it. */
  const by = $derived(summarizers?.transcribers.find((x) => x.id === settings?.transcribe_by));
  /** Switch the kind of transcript; a detailed one needs a transcriber that times its lines. */
  function setDetail(detail: MeetingSettings["detail"]) {
    if (!settings || settings.detail === detail) return;
    if (detail === "detailed" && by && !by.detailed) save({ detail, transcribe_by: "local" });
    else save({ detail });
  }
  const LANGUAGES = ["", "it", "en", "es", "fr", "de", "pt"];
  const WHEN = $derived([
    ["after", t.whenAfter],
    ["manual", t.whenManual],
    ["live", t.whenLive],
  ] as const);
</script>

{#if settings}
  {#if onmodels}
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{i18n.t.intelligence.link}</b><span>{i18n.t.intelligence.meetingsLinkHint}</span></div>
        <div class="how"><button class="btn small" onclick={onmodels}><Icon name="sparkles" size={14} />{i18n.t.intelligence.open}</button></div>
      </div>
    </section>
  {/if}
  <section class="panel">
    <div class="setting stack">
      <div class="what"><b>{t.detail}</b><span>{t.detailHint}</span></div>
      <div class="kinds" role="radiogroup" aria-label={t.detail}>
        <button class="kind" class:on={!simple} role="radio" aria-checked={!simple} title={by && !by.detailed ? t.detailedNeedsTimes : undefined} onclick={() => setDetail("detailed")}>
          <b>{t.detailed}</b>
          <span class="sample timed">
            {#each t.detailedExample as [at, who, said] (at)}<span class="mono faint">{at}</span><span class="who">{who}</span><span>{said}</span>{/each}
          </span>
          <span class="dim small">{t.detailedHint}</span>
        </button>
        <button class="kind" class:on={simple} role="radio" aria-checked={simple} onclick={() => setDetail("simple")}>
          <b>{t.simple}</b>
          <span class="sample">{t.simpleExample}</span>
          <span class="dim small">{t.simpleHint}</span>
        </button>
      </div>
    </div>
    {#if by && !by.ready}<div class="setting"><p class="error-text">{t.missingKey(serviceOf(by.id))}</p></div>{/if}
    {#if summarizers && !summarizers.brain && !settings.summarize_by}<div class="setting"><p class="error-text">{t.needSummarizer}</p></div>{/if}
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
    <div class="setting" class:off={simple || (by && !by.speakers)}>
      <div class="what"><b>{t.speakers}</b><span>{by && !by.speakers ? t.speakersNone : t.speakersHint}</span></div>
      <div class="how"><Toggle checked={settings.speakers} label={t.speakers} onchange={(on) => save({ speakers: on })} /></div>
    </div>
  </section>

  <h2 class="section-title">{t.stepSummary}</h2>
  <p class="dim note">{t.stepSummaryNote}</p>
  <section class="panel">
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
  .kinds {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 10px;
  }
  .kind {
    display: grid;
    align-content: start;
    gap: 8px;
    padding: 12px 14px;
    text-align: left;
    color: inherit;
    font: inherit;
    background: transparent;
    border: 1px solid var(--rim);
    border-radius: var(--r-md);
    cursor: pointer;
  }
  .kind:hover:not(:disabled) {
    background: var(--hover);
  }
  .kind.on {
    border-color: var(--pearl);
    box-shadow: inset 0 0 0 1px var(--pearl);
  }
  .kind:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .sample {
    padding: 8px 10px;
    border-radius: var(--r-sm);
    background: var(--hover);
    font-size: var(--fs-meta);
    line-height: 1.45;
  }
  .sample.timed {
    display: grid;
    grid-template-columns: auto auto 1fr;
    gap: 2px 8px;
  }
  .sample .who {
    font-weight: 600;
  }
  .off {
    opacity: 0.5;
  }
  .section-title + .panel,
  .note + .panel {
    margin-bottom: 22px;
  }
</style>
