<!-- The same explicit, reversible provider wizard in onboarding and Settings. -->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { PROFILES, applyProfile, keyOf, type ProfileId } from "../profiles";
  import { store } from "../store.svelte";
  import type { KeySource, VoiceInfo } from "../types";
  import KeyField from "./KeyField.svelte";

  let { compact = false, onchange }: { compact?: boolean; onchange?: () => void } = $props();
  const t = $derived(i18n.t.profiles);
  let voice = $state<VoiceInfo | null>(null);
  let selected = $state<ProfileId | null>(null);
  let stage = $state<1 | 2 | 3>(1);
  let busy = $state(false);
  let applied = $state(false);
  let error = $state("");
  let downloadError = $state("");
  const source = $derived<KeySource | null>(selected === "google" ? store.info?.google_key ?? null : selected === "openai" ? store.info?.openai_key ?? null : selected === "anthropic" ? store.info?.key ?? null : selected === "groq" ? store.info?.groq_key ?? null : null);
  const localVoiceMissing = $derived(!!voice && ["it_IT-paola-medium", "en_GB-alba-medium"].some((id) => !voice!.models.some((m) => m.id === id && m.installed)));
  const localHearingMissing = $derived(!!voice && !voice.models.some((m) => m.id === (voice!.recommendation?.whisper ?? "small") && m.installed));
  let meetingModel = $state<boolean | null>(null);
  let downloading = $state(false);

  async function read() {
    voice = await call<VoiceInfo>("voice_info").catch(() => voice);
    meetingModel = await call<boolean>("meeting_model").catch(() => null);
  }
  $effect(() => { void read(); });
  async function keyed() {
    await store.refreshInfo();
    await read();
    onchange?.();
  }
  async function apply() {
    if (!selected || !voice || !keyOf(selected)) return;
    busy = true;
    error = "";
    try {
      voice = await applyProfile(selected, voice);
      applied = true;
      onchange?.();
    } catch (e) {
      error = why(e);
      await store.refreshInfo().catch(() => {});
      await read();
    } finally { busy = false; }
  }
  async function downloadLocal() {
    if (!selected || !voice) return;
    downloading = true;
    downloadError = "";
    try {
      if (selected !== "google" && meetingModel === false) await call("meeting_model_download");
      if (selected === "anthropic" && localHearingMissing) await call("download_model", { id: voice.recommendation?.whisper ?? "small" });
      if (selected !== "google") {
        if (!voice.piper) await call("install_engine", { engine: "piper" });
        for (const id of ["it_IT-paola-medium", "en_GB-alba-medium"]) {
          if (!voice.models.some((m) => m.id === id && m.installed)) await call("download_model", { id });
        }
      }
    } catch (e) { downloadError = why(e); }
    finally { await read(); downloading = false; }
  }
</script>

<div class="wizard">
  <p class="dim small">{t.step(stage)} · {stage === 1 ? t.choose : stage === 2 ? t.keyStep : t.review}</p>
  {#if stage === 1}
    <p class="dim small">{t.intro}</p>
    <section class="tiles profiles" aria-label={t.choose}>
      {#each PROFILES as id (id)}
        <button class="tile" class:on={selected === id} onclick={() => { selected = id; applied = false; stage = 2; }}>
          <b>{t.names[id]}</b><span>{t.notes[id]}</span>
        </button>
      {/each}
    </section>
    <p class="dim small">{t.advancedHint}</p>
  {:else if selected && stage === 2}
    <h3>{t.needsKey(selected)}</h3>
    <p class="dim small">{t.keyExplanation}</p>
    <KeyField provider={selected} {source} onchange={keyed} />
    <div class="actions"><button class="btn" onclick={() => (stage = 1)}>{t.back}</button><button class="btn primary" disabled={!source} onclick={() => (stage = 3)}>{t.next}</button></div>
  {:else if selected}
    <h3>{t.names[selected]}</h3>
    <p class="dim small">{t.reviewIntro}</p>
    <ul class="review">
      <li>{t.plan[selected]}</li>
      {#if selected !== "google"}<li>{t.localMeetings}{#if meetingModel === false} {t.downloadMeeting}{/if}</li>{/if}
      {#if selected === "anthropic"}<li>{t.localHearing}{#if localHearingMissing} {t.downloadVoice}{/if}</li>{/if}
      {#if selected !== "google"}<li>{t.localSpeech}{#if localVoiceMissing || !voice?.piper} {t.downloadVoice}{/if}</li>{:else}<li>{t.googleFallback}</li>{/if}
      <li>{t.boardLimit}</li>
      <li>{t.costPrivacy}</li>
    </ul>
    {#if selected !== "google" && (meetingModel === false || localVoiceMissing || !voice?.piper || (selected === "anthropic" && localHearingMissing))}
      <button class="btn" disabled={downloading || !voice} onclick={downloadLocal}>{downloading ? t.downloading : t.downloadLocal}</button>
    {/if}
    {#if downloadError}<p class="error-text">{t.downloadError} {downloadError}</p>{/if}
    {#if applied}<p class="status"><span class="dot done"></span>{t.applied}</p>{/if}
    {#if error}<p class="error-text">{t.partial} {error}</p>{/if}
    <div class="actions"><button class="btn" disabled={busy} onclick={() => (stage = 2)}>{t.back}</button><button class="btn primary" disabled={busy || !voice || !source} onclick={apply}>{busy ? t.applying : t.apply}</button></div>
    {#if compact}<p class="dim small">{t.continueSetup}</p>{/if}
  {/if}
</div>

<style>
  .wizard { display: grid; gap: 12px; }
  .profiles { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .small { font-size: var(--fs-meta); }
  .review { display: grid; gap: 8px; padding-left: 20px; color: var(--haze); font-size: var(--fs-body); }
  .actions { display: flex; gap: 8px; justify-content: flex-end; }
  .status { display: flex; align-items: center; gap: 8px; }
</style>
