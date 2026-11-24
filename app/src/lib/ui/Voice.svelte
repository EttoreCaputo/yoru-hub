<!--
  Yoru's voice: whether he listens, whether he answers aloud, and how a
  conversation goes (follow-ups). Who listens and with which voice he
  answers, with their models and keys, are in Settings > Intelligence; the
  board's own (wake word, volume, chime) are on its page.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import type { VoiceInfo, VoiceSettings } from "../types";
  import Icon from "./Icon.svelte";
  import Toggle from "./Toggle.svelte";

  let { onmodels }: { onmodels?: (e: Event) => void } = $props();

  const t = $derived(i18n.t.board);
  const ti = $derived(i18n.t.intelligence);
  let info = $state<VoiceInfo | null>(null);
  let error = $state("");

  const read = async () => (info = await call<VoiceInfo>("voice_info").catch(() => info));
  $effect(() => {
    void read();
  });

  async function set(change: Partial<VoiceSettings>) {
    if (!info) return;
    error = "";
    try {
      info = await call<VoiceInfo>("set_voice", { settings: { ...$state.snapshot(info.settings), ...change } });
    } catch (e) {
      error = why(e);
    }
  }

  const LANGS = ["it", "en"];
  const serviceOf = (id: string | undefined) => (!id ? null : id.includes(":") ? id.slice(0, id.indexOf(":")) : "local");
  const nameOf = (svc: string) => (svc === "local" ? t.local : (info?.providers.find((p) => p.id === svc)?.name ?? svc));
  const heard = $derived(info?.settings.enabled ? serviceOf(info.settings.model) : null);
  const spoken = $derived(info?.settings.speak ? serviceOf(info.settings.voices[i18n.lang] ?? Object.values(info.settings.voices)[0]) : null);

  // Answering aloud again keeps the voices picked before; with none yet, this Mac's first for each language.
  function speakOn() {
    if (!info) return;
    if (Object.values(info.settings.voices).some(Boolean)) return set({ speak: true });
    const voices = { ...info.settings.voices };
    for (const l of LANGS) {
      const first = info.models.find((m) => m.kind === "voice" && serviceOf(m.id) === "local" && m.language === l);
      if (first) voices[l] = first.id;
    }
    set({ speak: true, voices });
  }
</script>

{#if info}
  {@const s = info.settings}
  <section class="panel">
    <div class="setting"><div class="what"><b>{t.currentVoice}</b><span>{s.live ? t.currentLive(s.live) : t.currentPipeline(heard ? nameOf(heard) : t.off, spoken ? nameOf(spoken) : t.off)}</span></div></div>
    {#if !s.live && info.stt_error}<div class="setting"><p class="error-text">{info.stt_error}</p></div>{/if}
    {#if !s.live && info.tts_error}<div class="setting"><p class="error-text">{info.tts_error}</p></div>{/if}
  </section>
  {#if s.live}
    <section class="panel active live-on">
      <div class="setting">
        <div class="what"><b>{t.liveOnTitle(s.live)}</b><span class="dim">{t.liveOnHint}</span></div>
      </div>
    </section>
  {:else}
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.listensToggle}</b><span>{s.enabled ? t.listenHint : t.listensOffHint}</span></div>
        <div class="how"><Toggle checked={s.enabled} label={t.listensToggle} onchange={(on) => set({ enabled: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.speaksToggle}</b><span>{s.speak ? t.speaksHint : t.speaksOffHint}</span></div>
        <div class="how"><Toggle checked={s.speak} label={t.speaksToggle} onchange={(on) => (on ? speakOn() : set({ speak: false }))} /></div>
      </div>
    </section>
  {/if}

  {#if s.speak || s.live}
    <h2 class="section-title">{t.conversation}</h2>
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.followUp}</b></div>
        <div class="how"><Toggle checked={s.follow_up} label={t.followUp} onchange={(on) => set({ follow_up: on })} /></div>
      </div>
    </section>
  {/if}

  {#if onmodels}
    <section class="panel link">
      <div class="setting">
        <div class="what"><b>{ti.link}</b><span>{ti.voiceLinkHint}</span></div>
        <div class="how"><button class="btn small" onclick={onmodels}><Icon name="sparkles" size={14} />{ti.open}</button></div>
      </div>
    </section>
  {/if}
  <p class="dim note">{t.onTheBoard}</p>
  {#if error}<p class="error-text note">{error}</p>{/if}
{/if}

<style>
  .note {
    margin-top: 12px;
  }
  .link {
    margin-top: 22px;
  }
</style>
