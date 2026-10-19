<!--
  The first run, in four steps: Yoru on the desk, its brain (Groq, free, or
  Claude, and its key) with its voice as an optional part (an ElevenLabs
  key and voice), a first service, and a few lines about you (the first
  memory). Every step can be skipped; Settings can run it again.
-->
<script lang="ts">
  import { call, why } from "./api";
  import { i18n } from "./i18n/index.svelte";
  import { store } from "./store.svelte";
  import type { AccountVoice, Brain, VoiceInfo } from "./types";
  import KeyField from "./ui/KeyField.svelte";
  import ServiceIcon from "./ui/ServiceIcon.svelte";
  import YoruBoard from "./YoruBoard.svelte";

  const t = $derived(i18n.t.setup);
  let step = $state(0);
  /** The optional part of the brain step: Yoru's voice. */
  let voicePart = $state(false);
  let about = $state("");
  let busy = $state<string | null>(null);
  let error = $state("");

  // One click each: the app's own clients first (mail and calendar matter most to an assistant), then dynamic ones.
  const quick = $derived([...store.presets.filter((p) => p.app_client), ...store.presets.filter((p) => p.registration === "dynamic")].slice(0, 4));
  const connected = $derived(new Set(store.connectors.filter((c) => c.signed_in || c.state === "connected").map((c) => c.id)));

  async function connect(id: string) {
    busy = id;
    error = "";
    try {
      if (!store.connectors.some((c) => c.id === id)) await call("add_connector", { config: null, preset: id, clientId: null, clientSecret: null });
      await call("login_connector", { id });
    } catch (e) {
      error = why(e);
    } finally {
      busy = null;
      await store.refreshConnectors();
    }
  }

  // The brain: the settings' choice, else the one with a key, else Groq (free, to start).
  const brain = $derived<Brain>(store.info?.settings.provider ?? store.info?.brain ?? "groq");
  const brainKey = $derived(brain === "groq" ? (store.info?.groq_key ?? null) : (store.info?.key ?? null));
  const pick = (b: Brain) => store.saveSettings((s) => (s.provider = b));

  // The voice: an ElevenLabs key, then one of the account's voices.
  let voice = $state<VoiceInfo | null>(null);
  let voices = $state<AccountVoice[] | null>(null);
  let chosenVoice = $state("");
  let voiceBusy = $state(false);
  let voiceError = $state("");
  const elevenKey = $derived(voice?.providers.find((p) => p.id === "elevenlabs")?.key ?? null);
  const usedVoice = $derived(voices?.find((v) => voice?.settings.voices.it === `elevenlabs:voice/${v.voice_id}`) ?? null);

  async function readVoice() {
    voice = await call<VoiceInfo>("voice_info").catch(() => voice);
    if (!voice?.providers.find((p) => p.id === "elevenlabs")?.key) return;
    voices = null;
    try {
      voices = await call<AccountVoice[]>("elevenlabs_voices");
      chosenVoice = usedVoice?.voice_id ?? voices[0]?.voice_id ?? "";
    } catch (e) {
      voiceError = why(e);
      voices = [];
    }
  }
  $effect(() => {
    if (voicePart) void readVoice();
  });

  /** The same voice for both languages: ElevenLabs' speak either. */
  async function useVoice() {
    if (!voice || !chosenVoice) return;
    voiceBusy = true;
    voiceError = "";
    try {
      const id = `elevenlabs:voice/${chosenVoice}`;
      voice = await call<VoiceInfo>("set_voice", { settings: { ...$state.snapshot(voice.settings), speak: true, voices: { ...voice.settings.voices, it: id, en: id } } });
    } catch (e) {
      voiceError = why(e);
    } finally {
      voiceBusy = false;
    }
  }

  async function testVoice() {
    voiceBusy = true;
    voiceError = "";
    try {
      await call("test_voice", { board: null, language: i18n.lang });
    } catch (e) {
      voiceError = why(e);
    } finally {
      voiceBusy = false;
    }
  }

  function next() {
    if (step === 1 && !voicePart) voicePart = true;
    else (voicePart = false), step++;
  }

  function back() {
    if (voicePart) voicePart = false;
    else if (step === 2) (step = 1), (voicePart = true);
    else step--;
  }

  /** The lines go under "About you" in memory.md: Yoru reads them every turn. */
  async function finish() {
    const text = about.trim();
    if (text) {
      const memory = await call<string>("memory").catch(() => "");
      const head = "## About you";
      const at = memory.indexOf(head);
      const lines = text.split("\n").map((l) => (l.trim() ? `- ${l.trim().replace(/^[-*]\s*/, "")}` : "")).filter(Boolean).join("\n");
      const next = at < 0 ? `${memory.trimEnd()}\n\n${head}\n\n${lines}\n` : `${memory.slice(0, at + head.length)}\n\n${lines}${memory.slice(at + head.length).replace(/^\n+/, "\n\n")}`;
      await call("set_memory", { text: next }).catch(() => {});
    }
    await store.finishOnboarding();
  }
</script>

<div class="veil" role="dialog" aria-modal="true" aria-labelledby="setup-title">
  <div class="sheet">
    <ol class="steps" aria-label="Setup">
      {#each t.steps as s, i (i)}
        <li class:on={i === step} class:past={i < step}><span class="n">{i + 1}</span>{s}</li>
      {/each}
    </ol>

    <div class="body">
      {#if step === 0}
        <div class="mirror"><YoruBoard board={store.boardState === "off" ? "off" : "live"} size={130} /></div>
        <h1 id="setup-title">{t.board.title}</h1>
        <p class="dim">{t.board.text}</p>
        <p class="status"><span class="dot {store.boardState === 'live' ? 'done' : 'think'}"></span>{store.boardState === "live" ? t.board.found : t.board.waiting}</p>
        {#if store.update}<p class="faint">{t.board.update(store.update.to)}</p>{/if}
      {:else if step === 1 && !voicePart}
        <h1 id="setup-title">{t.brain.title}</h1>
        <p class="dim">{t.brain.text}</p>
        <div class="services" role="radiogroup" aria-label={t.brain.title}>
          {#each [["groq", t.brain.groq, t.brain.groqWhat], ["anthropic", t.brain.anthropic, t.brain.anthropicWhat]] as [id, name, what] (id)}
            <button class="service" class:done={brain === id} role="radio" aria-checked={brain === id} onclick={() => pick(id as Brain)}>
              <b>{name}</b>
              <span>{what}</span>
            </button>
          {/each}
        </div>
        <p class="faint">{brain === "groq" ? t.brain.groqKey : t.brain.anthropicKey}</p>
        {#key brain}
          <div class="field"><KeyField provider={brain} source={brainKey} placeholder={brain === "groq" ? "gsk_…" : "sk-ant-…"} onchange={() => store.refreshInfo()} /></div>
        {/key}
        {#if store.info?.ready && brainKey}<p class="status"><span class="dot done"></span>{t.brain.ok}</p>{/if}
      {:else if step === 1}
        <h1 id="setup-title">{t.voice.title} <span class="chip">{t.optional}</span></h1>
        <p class="dim">{t.voice.text}</p>
        <div class="field"><KeyField provider="elevenlabs" source={elevenKey} onchange={readVoice} /></div>
        {#if elevenKey}
          {#if voices === null}
            <p class="faint">{t.voice.loading}</p>
          {:else if voices.length}
            <div class="row">
              <select bind:value={chosenVoice} aria-label={t.voice.pick}>
                {#each voices as v (v.voice_id)}<option value={v.voice_id}>{v.name}{v.category ? ` · ${v.category}` : ""}</option>{/each}
              </select>
              <button class="btn" disabled={voiceBusy || !chosenVoice || usedVoice?.voice_id === chosenVoice} onclick={useVoice}>{t.voice.use}</button>
              <button class="btn quiet" disabled={voiceBusy || !usedVoice || store.boardState !== "live"} onclick={testVoice}>{t.voice.test}</button>
            </div>
            {#if usedVoice}<p class="status"><span class="dot done"></span>{t.voice.using(usedVoice.name)}</p>{/if}
            {#if usedVoice && store.boardState !== "live"}<p class="faint">{t.voice.noBoard}</p>{/if}
          {:else}
            <p class="faint">{t.voice.none}</p>
          {/if}
        {/if}
        {#if voiceError}<p class="error-text">{voiceError}</p>{/if}
      {:else if step === 2}
        <h1 id="setup-title">{t.service.title}</h1>
        <p class="dim">{t.service.text}</p>
        <div class="services">
          {#each quick as p (p.id)}
            <button class="service with-mark" class:done={connected.has(p.id)} disabled={!!busy} onclick={() => connect(p.id)}>
              <ServiceIcon id={p.id} name={p.name} url={p.url} size={30} />
              <b>{p.name}</b>
              <span class="what">{busy === p.id ? i18n.t.connectors.signingIn : connected.has(p.id) ? i18n.t.connectors.state.connected : p.notes}</span>
            </button>
          {/each}
        </div>
        {#if busy}<button class="btn quiet small" onclick={() => call("cancel_login", { id: busy })}>{i18n.t.connectors.cancel}</button>{/if}
        {#if error}<p class="error-text">{error}</p>{/if}
      {:else}
        <h1 id="setup-title">{t.you.title}</h1>
        <p class="dim">{t.you.text}</p>
        <textarea rows="5" bind:value={about} placeholder={t.you.placeholder}></textarea>
      {/if}
    </div>

    <footer>
      <button class="btn quiet" onclick={() => store.finishOnboarding()}>{t.skip}</button>
      <span class="spacer"></span>
      {#if step > 0}<button class="btn" onclick={back}>{t.back}</button>{/if}
      {#if step < 3}
        <button class="btn primary" onclick={next}>{step === 2 && !connected.size ? t.service.later : voicePart && !usedVoice ? t.voice.skip : t.next}</button>
      {:else}
        <button class="btn primary" onclick={finish}>{t.done}</button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .veil {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.82);
    backdrop-filter: blur(6px);
  }
  .sheet {
    width: min(620px, 100%);
    max-height: 100%;
    overflow: auto;
    border-radius: 24px;
    background: var(--ink);
    border: 1px solid var(--rim-2);
    box-shadow: 0 40px 120px -40px rgba(195, 190, 219, 0.25);
  }
  .steps {
    display: flex;
    gap: 18px;
    margin: 0;
    padding: 20px 28px;
    list-style: none;
    border-bottom: 1px solid var(--rim);
    font-size: 12.5px;
    color: var(--dusk);
  }
  .steps li {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .steps li.on {
    color: var(--pearl);
  }
  .steps li.past {
    color: var(--haze);
  }
  .n {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 1px solid currentColor;
    font: 600 10.5px/1 var(--sans);
  }
  .on .n {
    border-color: transparent;
    background: radial-gradient(circle at 34% 30%, #fff, var(--pearl-mid));
    color: var(--eye);
  }
  .body {
    display: grid;
    gap: 12px;
    padding: 28px;
    min-height: 300px;
    align-content: start;
  }
  .mirror {
    justify-self: start;
    margin: -8px 0 0 -14px;
  }
  .mirror :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  h1 {
    font: 600 26px/1.15 var(--display);
    letter-spacing: -0.01em;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .field {
    margin-top: 6px;
  }
  .services {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin-top: 6px;
  }
  .service {
    display: grid;
    gap: 4px;
    padding: 14px;
    border-radius: var(--r);
    border: 1px solid var(--rim);
    background: var(--ink-2);
    color: var(--pearl);
    text-align: left;
    font: inherit;
    cursor: pointer;
  }
  /* The mark beside the name, the notes under the name. */
  .service.with-mark {
    grid-template-columns: auto minmax(0, 1fr);
    column-gap: 12px;
    align-items: center;
  }
  .with-mark .what {
    grid-column: 2;
  }
  .service:hover:not(:disabled) {
    border-color: var(--rim-2);
  }
  .service span {
    color: var(--haze);
    font-size: 12.5px;
  }
  .service.done {
    border-color: color-mix(in srgb, var(--done) 40%, transparent);
  }
  .service[aria-checked="true"] {
    border-color: var(--pearl-mid);
    background: color-mix(in srgb, var(--pearl) 6%, var(--ink-2));
  }
  .chip {
    display: inline-block;
    vertical-align: middle;
    margin-left: 8px;
    padding: 3px 9px;
    border-radius: 999px;
    border: 1px solid var(--rim-2);
    color: var(--haze);
    font: 500 11px/1.2 var(--sans);
    letter-spacing: 0;
  }
  .row select {
    flex: 1;
    min-width: 0;
  }
  footer {
    display: flex;
    gap: 8px;
    padding: 16px 28px 20px;
    border-top: 1px solid var(--rim);
  }
  .spacer {
    flex: 1;
  }
</style>
