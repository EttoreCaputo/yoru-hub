<!--
  Yoru's voice: listening after the wake word (and with which speech-to-text
  model), answering aloud (and with which voice per language), the board's
  volume and chime. Downloads show their progress while they run.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { BoardVoice, ModelInfo, VoiceInfo, VoiceSettings } from "../types";
  import Toggle from "./Toggle.svelte";

  let { wakeWord }: { wakeWord: string | null } = $props();
  const t = $derived(i18n.t.board);
  let info = $state<VoiceInfo | null>(null);
  let board = $state<BoardVoice | null>(null);
  let error = $state("");

  const read = async () => (info = await call<VoiceInfo>("voice_info").catch(() => info));
  $effect(() => {
    void read();
  });
  $effect(() => {
    if (store.link !== "connected") return;
    call<BoardVoice>("board_voice", { board: null }).then((b) => (board = b), () => {});
  });
  // While something downloads or installs, follow it.
  $effect(() => {
    if (!info?.download && !info?.piper_install && info?.stt !== "starting" && info?.tts !== "starting") return;
    const id = setInterval(read, 700);
    return () => clearInterval(id);
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

  async function act(cmd: string, args: Record<string, unknown> = {}) {
    error = "";
    const job = call(cmd, args).catch((e) => (error = why(e)));
    setTimeout(read, 200);
    await job;
    await read();
  }

  const usable = (m: ModelInfo) => m.installed || !m.provider;
  const label = (m: ModelInfo) => `${m.provider ? `${m.provider} · ` : ""}${m.id.replace(/^[^:]+:/, "")}${m.note ? ` — ${m.note}` : ""}`;
  const whispers = $derived(info?.models.filter((m) => m.kind === "whisper" && usable(m)) ?? []);
  const voicesFor = (lang: string) => info?.models.filter((m) => m.kind === "voice" && (m.language === lang || !m.language) && usable(m)) ?? [];
  const model = (id: string | undefined) => info?.models.find((m) => m.id === id);
  const dl = $derived(info?.download ?? null);
  const sttModel = $derived(model(info?.settings.model));
  const dot = (s: string | undefined) => ({ ready: "done", starting: "think", error: "wrong" })[s ?? ""] ?? "";
</script>

{#if info}
  {@const s = info.settings}
  <section class="panel">
    <div class="setting">
      <div class="what">
        <b>{t.listen}</b>
        <span>{wakeWord ? t.wakeWord(wakeWord) : t.listenHint}</span>
      </div>
      <div class="how">
        {#if s.enabled}<span class="state"><span class="dot {dot(info.stt)}"></span>{t.state[info.stt]}</span>{/if}
        <Toggle checked={s.enabled} label={t.listen} onchange={(on) => set({ enabled: on })} />
      </div>
    </div>
    {#if s.enabled}
      <div class="setting">
        <div class="what"><b>{t.sttModel}</b>{#if info.stt_error}<span class="error-text">{info.stt_error}</span>{/if}</div>
        <div class="how">
          <select value={s.model} onchange={(e) => set({ model: e.currentTarget.value })} aria-label={t.sttModel}>
            {#each whispers as m (m.id)}<option value={m.id}>{label(m)}</option>{/each}
          </select>
          {#if sttModel && !sttModel.installed && !sttModel.provider}
            {#if dl?.[0] === sttModel.id}
              <span class="dim">{t.downloading(Math.round(dl[1]))}</span>
            {:else}
              <button class="btn small" disabled={!!dl} onclick={() => act("download_model", { id: sttModel.id })}>{t.download}</button>
            {/if}
          {/if}
        </div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.speak}</b>{#if info.tts_error}<span class="error-text">{info.tts_error}</span>{/if}</div>
        <div class="how">
          {#if s.speak}<span class="state"><span class="dot {dot(info.tts)}"></span>{t.state[info.tts]}</span>{/if}
          <Toggle checked={s.speak} label={t.speak} onchange={(on) => set({ speak: on })} />
        </div>
      </div>
      {#if s.speak}
        {#if !info.piper}
          <div class="setting">
            <div class="what"><b>Piper</b>{#if info.piper_install}<span>{info.piper_install}</span>{/if}</div>
            <div class="how"><button class="btn small" disabled={!!info.piper_install} onclick={() => act("install_engine", { engine: "piper" })}>{t.installPiper}</button></div>
          </div>
        {/if}
        {#each ["it", "en"] as lang (lang)}
          {@const v = model(s.voices[lang])}
          <div class="setting">
            <div class="what"><b>{t.voices}</b><span>{t.voiceFor(lang)}</span></div>
            <div class="how">
              <select value={s.voices[lang] ?? ""} onchange={(e) => set({ voices: { ...s.voices, [lang]: e.currentTarget.value } })} aria-label={t.voiceFor(lang)}>
                {#each voicesFor(lang) as m (m.id)}<option value={m.id}>{label(m)}</option>{/each}
              </select>
              {#if v && !v.installed && !v.provider}
                {#if dl?.[0] === v.id}
                  <span class="dim">{t.downloading(Math.round(dl[1]))}</span>
                {:else}
                  <button class="btn small" disabled={!!dl} onclick={() => act("download_model", { id: v.id })}>{t.download}</button>
                {/if}
              {/if}
            </div>
          </div>
        {/each}
        <div class="setting">
          <div class="what"><b>{t.followUp}</b></div>
          <div class="how"><Toggle checked={s.follow_up} label={t.followUp} onchange={(on) => set({ follow_up: on })} /></div>
        </div>
      {/if}
    {/if}
    {#if board}
      {#if board.volume !== null}
        <div class="setting">
          <div class="what"><b>{t.volume}</b></div>
          <div class="how">
            <input type="range" min="0" max="100" value={board.volume} onchange={(e) => call("set_board_volume", { board: null, percent: Number(e.currentTarget.value) })} aria-label={t.volume} />
            <button class="btn quiet small" disabled={info.tts !== "ready"} onclick={() => act("test_voice", { board: null, language: i18n.lang })}>{t.test}</button>
          </div>
        </div>
      {/if}
      {#if board.wake_sound !== null}
        <div class="setting">
          <div class="what"><b>{t.wakeSound}</b></div>
          <div class="how"><Toggle checked={board.wake_sound} label={t.wakeSound} onchange={(on) => ((board!.wake_sound = on), call("set_board_wake_sound", { board: null, on }))} /></div>
        </div>
      {/if}
    {/if}
    {#if error}<p class="error-text">{error}</p>{/if}
  </section>
{/if}

<style>
  select {
    width: auto;
    max-width: 320px;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--haze);
    font-size: 12px;
    margin-right: 6px;
  }
  input[type="range"] {
    width: 160px;
  }
</style>
