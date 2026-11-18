<!--
  Yoru's voice, laid out as the brain is: tiles pick the service he listens
  with and the one he speaks with (or neither), then his voice in each
  language, how a conversation goes (follow-ups, the board's volume and
  chime) and Gemini Live. The models that transcribe and judge a turn are
  advanced; the services' keys are in Settings' keys page. Local models need
  a download, which shows its progress while it runs.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { AccountVoice, BoardVoice, ModelInfo, SidecarState, VoiceInfo, VoiceSettings, YoruSettings } from "../types";
  import Toggle from "./Toggle.svelte";

  const t = $derived(i18n.t.board);
  let info = $state<VoiceInfo | null>(null);
  let board = $state<BoardVoice | null>(null);
  let error = $state("");

  const read = async () => (info = await call<VoiceInfo>("voice_info").catch(() => info));
  $effect(() => {
    void read();
  });
  // The wake word is one of the board's own settings.
  let wakeWord = $state<string | null>(null);
  $effect(() => {
    if (store.link !== "connected") return;
    void store.boardId;
    call<BoardVoice>("board_voice", { board: null }).then((b) => (board = b), () => {});
    call<YoruSettings>("yoru_settings", { board: null }).then((s) => (wakeWord = typeof s?.wake_word === "string" ? s.wake_word : null), () => {});
  });
  // While something downloads or installs, follow it.
  $effect(() => {
    if (!info?.download && !info?.piper_install && info?.stt !== "starting" && info?.tts !== "starting") return;
    const id = setInterval(read, 700);
    return () => clearInterval(id);
  });

  // The ElevenLabs account's voices aren't in the catalog: ask for them
  // once its key is there. They speak any language, so each one fits both.
  let account = $state<AccountVoice[]>([]);
  const elevenKey = $derived(!!info?.providers.find((p) => p.id === "elevenlabs")?.key);
  $effect(() => {
    if (!elevenKey) return void (account = []);
    call<AccountVoice[]>("elevenlabs_voices").then((v) => (account = v), (e) => (error = why(e)));
  });
  const ACCOUNT = "elevenlabs:voice/";
  const accountId = (v: AccountVoice) => `${ACCOUNT}${v.voice_id}`;

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

  // A service is "local" or a provider's id; a model's comes from its id.
  const LOCAL = "local";
  const LANGS = ["it", "en"];
  const serviceOf = (id: string | undefined) => (!id ? null : id.includes(":") ? id.slice(0, id.indexOf(":")) : LOCAL);
  const keyed = (svc: string) => svc === LOCAL || !!info?.providers.find((p) => p.id === svc)?.key;
  const nameOf = (svc: string) => (svc === LOCAL ? t.local : (info?.providers.find((p) => p.id === svc)?.name ?? svc));
  const noteOf = (svc: string) => t.serviceNotes[svc] ?? info?.providers.find((p) => p.id === svc)?.note ?? "";

  // `elevenlabs:voice` only stands for the account's voices, listed apart.
  const whispers = $derived(info?.models.filter((m) => m.kind === "whisper") ?? []);
  const voices = $derived(info?.models.filter((m) => m.kind === "voice" && m.id !== "elevenlabs:voice") ?? []);
  const of = (list: ModelInfo[], svc: string) => list.filter((m) => serviceOf(m.id) === svc);
  const uniq = (ids: (string | null)[]) => [...new Set(ids.filter((s): s is string => !!s))].sort((a, b) => (a === LOCAL ? -1 : b === LOCAL ? 1 : 0));
  const hearing = $derived(uniq(whispers.map((m) => serviceOf(m.id))));
  const speaking = $derived(uniq([...voices.map((m) => serviceOf(m.id)), ...(info?.providers.some((p) => p.id === "elevenlabs") ? ["elevenlabs"] : [])]));
  /** A service's models of `list` for a picker, grouped by service: those with a key, and the one in use. */
  const groups = (services: string[], list: (svc: string) => { id: string; label: string }[], current: string | undefined) =>
    services.filter((svc) => keyed(svc) || serviceOf(current) === svc).map((svc) => ({ svc, options: list(svc) })).filter((g) => g.options.length);

  /** A service's voices for a language: the catalog's, and for ElevenLabs the account's. */
  function voicesIn(svc: string, lang: string): { id: string; label: string }[] {
    const list = of(voices, svc)
      .filter((m) => m.language === lang || !m.language)
      .map((m) => ({ id: m.id, label: label(m) }));
    if (svc === "elevenlabs") list.push(...account.map((a) => ({ id: accountId(a), label: `${a.name}${a.category ? ` · ${a.category}` : ""}` })));
    return list;
  }
  const model = (id: string | undefined) => info?.models.find((m) => m.id === id);
  const mb = (m: ModelInfo) => Math.round(m.bytes / 1e6);
  const missing = (m: ModelInfo | undefined) => !!m && !m.installed && !m.provider;
  const label = (m: ModelInfo) => `${m.id.replace(/^[^:]+:/, "")}${m.note ? ` — ${m.note}` : ""}${missing(m) ? ` · ${t.toDownload(mb(m))}` : ""}`;
  const dl = $derived(info?.download ?? null);
  const intents = $derived(info?.models.filter((m) => m.kind === "intent") ?? []);
  const turns = $derived(info?.models.filter((m) => m.kind === "turn") ?? []);

  // What each row of tiles shows as picked. Speaking follows the app's language's voice.
  const heard = $derived(info?.settings.enabled ? serviceOf(info.settings.model) : null);
  const spoken = $derived(info?.settings.speak ? serviceOf(info.settings.voices[i18n.lang] ?? Object.values(info.settings.voices)[0]) : null);
  const canSpeak = (svc: string) => keyed(svc) && LANGS.some((l) => voicesIn(svc, l).length);

  function hearWith(svc: string | null) {
    if (!svc || !info) return set({ enabled: false });
    const keep = serviceOf(info.settings.model) === svc;
    set({ enabled: true, model: keep ? info.settings.model : (of(whispers, svc)[0]?.id ?? info.settings.model) });
  }
  // Each language takes the service's first voice for it, unless it has one there already.
  function speakWith(svc: string | null) {
    if (!svc || !info) return set({ speak: false });
    const now = info.settings.voices;
    const next = { ...now };
    for (const l of LANGS) {
      const list = voicesIn(svc, l);
      if (list.length && serviceOf(now[l]) !== svc) next[l] = list[0].id;
    }
    set({ speak: true, voices: next });
  }
</script>

{#if info}
  {@const s = info.settings}
  {@const sttModel = model(s.model)}
  {#if s.live}
    <section class="panel active live-on">
      <div class="setting">
        <div class="what"><b>{t.liveOnTitle(s.live)}</b><span class="dim">{t.liveOnHint}</span></div>
        <div class="how"><button class="btn small" onclick={() => set({ live: "" })}>{t.liveTurnOff}</button></div>
      </div>
    </section>
  {/if}
  <h2 class="section-title first">{t.listenWith}</h2>
  <section class="tiles" aria-label={t.listenWith}>
    {@render tile(null, heard === null, t.offListen, info.stt, hearWith)}
    {#each hearing as svc (svc)}{@render tile(svc, heard === svc, noteOf(svc), info.stt, hearWith)}{/each}
  </section>
  {#if s.enabled}<p class="dim note">{wakeWord ? t.wakeWord(wakeWord) : t.listenHint}</p>{/if}
  {#if s.enabled && sttModel && missing(sttModel)}
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.sttToDownload}</b><span class="mono">{sttModel.id}</span></div>
        <div class="how">{@render fetch(sttModel)}</div>
      </div>
    </section>
  {/if}
  {#if info.stt_error}<p class="error-text note">{info.stt_error}</p>{/if}

  <h2 class="section-title">{t.speakWith}</h2>
  <section class="tiles" aria-label={t.speakWith}>
    {@render tile(null, spoken === null, t.offSpeak, info.tts, speakWith)}
    {#each speaking as svc (svc)}{@render tile(svc, spoken === svc, noteOf(svc), info.tts, speakWith, !canSpeak(svc))}{/each}
  </section>
  {#if [...hearing, ...speaking].some((svc) => !keyed(svc))}<p class="dim note">{t.addKeys}</p>{/if}
  {#if info.tts_error}<p class="error-text note">{info.tts_error}</p>{/if}
  {#if s.speak}
    <section class="panel" class:active={!!spoken}>
      {#each LANGS as lang (lang)}
        {@const m = model(s.voices[lang])}
        {@const options = groups(speaking, (svc) => voicesIn(svc, lang), s.voices[lang])}
        <div class="setting">
          <div class="what"><b>{t.voiceFor(lang)}</b></div>
          <div class="how">
            {#if m && missing(m)}{@render fetch(m)}{/if}
            <select value={s.voices[lang] ?? ""} onchange={(e) => set({ speak: true, voices: { ...s.voices, [lang]: e.currentTarget.value } })} aria-label={t.voiceFor(lang)}>
              {#if !s.voices[lang]}<option value="" disabled>—</option>{/if}
              {#each options as g (g.svc)}
                <optgroup label={nameOf(g.svc)}>{#each g.options as v (v.id)}<option value={v.id}>{v.label}</option>{/each}</optgroup>
              {/each}
              {#if s.voices[lang]?.startsWith(ACCOUNT) && !account.length}<option value={s.voices[lang]}>{s.voices[lang].slice(ACCOUNT.length)}</option>{/if}
            </select>
          </div>
        </div>
      {/each}
      {#if spoken === LOCAL && !info.piper}
        <div class="setting">
          <div class="what"><b>Piper</b>{#if info.piper_install}<span>{info.piper_install}</span>{/if}</div>
          <div class="how"><button class="btn small" disabled={!!info.piper_install} onclick={() => act("install_engine", { engine: "piper" })}>{t.installPiper}</button></div>
        </div>
      {/if}
    </section>
  {/if}

  {#if s.speak || board?.volume != null || board?.wake_sound != null}
    <h2 class="section-title">{t.conversation}</h2>
    <section class="panel">
      {#if s.speak}
        <div class="setting">
          <div class="what"><b>{t.followUp}</b></div>
          <div class="how"><Toggle checked={s.follow_up} label={t.followUp} onchange={(on) => set({ follow_up: on })} /></div>
        </div>
      {/if}
      {#if board && board.volume !== null}
        <div class="setting">
          <div class="what"><b>{t.volume}</b></div>
          <div class="how">
            <input type="range" min="0" max="100" value={board.volume} onchange={(e) => call("set_board_volume", { board: null, percent: Number(e.currentTarget.value) })} aria-label={t.volume} />
            <button class="btn quiet small" disabled={info.tts !== "ready"} onclick={() => act("test_voice", { board: null, language: i18n.lang })}>{t.test}</button>
          </div>
        </div>
      {/if}
      {#if board && board.wake_sound !== null}
        <div class="setting">
          <div class="what"><b>{t.wakeSound}</b></div>
          <div class="how"><Toggle checked={board.wake_sound} label={t.wakeSound} onchange={(on) => ((board!.wake_sound = on), call("set_board_wake_sound", { board: null, on }))} /></div>
        </div>
      {/if}
    </section>
  {/if}

  {#if s.enabled}
    <details class="more">
      <summary class="section-title">{i18n.t.settings.advanced}</summary>
      <p class="dim hint">{t.advancedHint}</p>
      <section class="panel">
        <div class="setting">
          <div class="what"><b>{t.sttModel}</b></div>
          <div class="how">
            {#if sttModel && missing(sttModel)}{@render fetch(sttModel)}{/if}
            <select value={s.model} onchange={(e) => set({ enabled: true, model: e.currentTarget.value })} aria-label={t.sttModel}>
              {#each groups(hearing, (svc) => of(whispers, svc).map((h) => ({ id: h.id, label: label(h) })), s.model) as g (g.svc)}
                <optgroup label={nameOf(g.svc)}>{#each g.options as h (h.id)}<option value={h.id}>{h.label}</option>{/each}</optgroup>
              {/each}
            </select>
          </div>
        </div>
        {@render picker(t.turnModel, t.turnHint, s.turn_model, t.turnPause, turns, (id) => set({ turn_model: id }))}
        {#if s.speak && s.follow_up}
          {@render picker(t.intentModel, t.intentHint, s.intent_model, t.intentPhrases, intents, (id) => set({ intent_model: id }), !info.llm_server ? t.needsLlamaServer : "")}
        {/if}
      </section>
    </details>
  {/if}
  {#if error}<p class="error-text note">{error}</p>{/if}
{/if}

<!-- A service to pick (null: none), with what it's like, or why it can't be picked. -->
{#snippet tile(svc: string | null, on: boolean, note: string, state: SidecarState, pick: (svc: string | null) => void, blocked = svc !== null && !keyed(svc))}
  <button class="tile" class:on disabled={blocked} onclick={() => pick(svc)}>
    <b>{svc ? nameOf(svc) : t.off}</b>
    <span>{note}</span>
    {#if blocked}<span class="tag dim">{keyed(svc!) ? t.noVoices : t.needsKey(nameOf(svc!))}</span>
    {:else if on && svc}<span class="tag" class:dim={state !== "ready"}>{state === "ready" ? t.inUse : t.state[state]}</span>{/if}
  </button>
{/snippet}

<!-- One job a small local model can do, or the built-in way ("" picks it). -->
{#snippet picker(name: string, hint: string, value: string, builtIn: string, list: ModelInfo[], pick: (id: string) => void, blocked = "")}
  {@const m = model(value)}
  <div class="setting">
    <div class="what"><b>{name}</b><span class="dim">{blocked || hint}</span></div>
    <div class="how">
      {#if m && missing(m)}{@render fetch(m)}{/if}
      <select {value} onchange={(e) => pick(e.currentTarget.value)} aria-label={name}>
        <option value="">{builtIn}</option>
        {#each list as option (option.id)}<option value={option.id} disabled={!!blocked}>{label(option)}</option>{/each}
      </select>
    </div>
  </div>
{/snippet}

<!-- A picked local model still to download. -->
{#snippet fetch(m: ModelInfo)}
  {#if dl?.[0] === m.id}
    <span class="dim">{t.downloading(Math.round(dl[1]))}</span>
  {:else}
    <button class="btn primary small" disabled={!!dl} onclick={() => act("download_model", { id: m.id })}>{t.download} · {mb(m)} MB</button>
  {/if}
{/snippet}

<style>
  .section-title.first {
    margin-top: 0;
  }
  .note {
    margin-top: 12px;
  }
  .hint {
    margin: -4px 0 10px;
    font-size: var(--fs-meta);
  }
  select {
    max-width: 340px;
  }
  input[type="range"] {
    width: 160px;
  }
</style>
