<!--
  The first run: a guided tour over the whole window, Yoru's night (the
  app icon's sky, an aurora and stars) with Yoru himself as the guide. He
  floats on the left, follows the pointer, says each step in a bubble that
  types itself and reacts (a wave, curiosity, joy when the microphone is
  given); each step glides in from the side it comes from, its parts one
  after the other. Six steps: the language, the Mac's permissions (the
  microphone is required: nothing goes on without it, and the step stays
  once it's given, for the others) with Yoru in the notch and noticing
  meetings, Yoru on the desk, the brain (a provider and its key), a first
  service, and a few lines about you (the first memory). Once the
  microphone is Yoru's every other step can be skipped; Settings can run
  it again. Always the dark look: it's Yoru's night, whatever the theme.
-->
<script lang="ts">
  import { cubicOut } from "svelte/easing";
  import { call, why } from "./api";
  import { i18n, LANGUAGES, type Lang } from "./i18n/index.svelte";
  import { store } from "./store.svelte";
  import type { KeySource, MeetingSettings, PermissionKind, VoiceInfo } from "./types";
  import PermissionList from "./ui/PermissionList.svelte";
  import Toggle from "./ui/Toggle.svelte";
  import KeyField from "./ui/KeyField.svelte";
  import { PROFILES, applyProfile, keyOf, prepareLocal, type ProfileId } from "./profiles";
  import ServiceIcon from "./ui/ServiceIcon.svelte";
  import YoruBoard from "./YoruBoard.svelte";

  const t = $derived(i18n.t.setup);
  let step = $state(0);
  /** 1 going forward, −1 back: where the next step glides in from. */
  let dir = $state(1);
  let about = $state("");
  let busy = $state<string | null>(null);
  let error = $state("");
  /** Finishing: the night opens and lets the app through. */
  let leaving = $state(false);
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  // One click each: the app's own clients first (mail and calendar matter most to an assistant), then dynamic ones.
  const quick = $derived([...store.presets.filter((p) => p.app_client), ...store.presets.filter((p) => p.registration === "dynamic")].slice(0, 4));
  const connected = $derived(new Set(store.connectors.filter((c) => c.signed_in || c.state === "connected").map((c) => c.id)));

  async function connect(id: string) {
    busy = id;
    error = "";
    try {
      if (!store.connectors.some((c) => c.id === id)) await call("add_connector", { config: null, preset: id, clientId: null, clientSecret: null });
      await call("login_connector", { id });
      react("happy");
    } catch (e) {
      error = why(e);
    } finally {
      busy = null;
      await store.refreshConnectors();
    }
  }

  const LAST = 5;
  const LANG_CHOICES: (Lang | null)[] = [null, ...LANGUAGES.map((l) => l.id)];
  const langName = (id: Lang) => LANGUAGES.find((l) => l.id === id)?.name ?? id;

  // Yoru in the notch and noticing meetings start on: they are what makes
  // him useful on the Mac, and the switches below say so before Next.
  let notch = $state(true);
  let meetings = $state<MeetingSettings | null>(null);
  call<MeetingSettings>("meeting_settings").then((m) => (meetings = { ...m, detect: true }), () => {});

  /** The choices on the Mac, kept when leaving the permissions step. */
  async function saveMac() {
    await call("set_notch", { on: notch }).catch(() => {});
    if (meetings) {
      const current = await call<MeetingSettings>("meeting_settings").catch(() => null);
      if (current) await call("set_meeting_settings", { settings: { ...current, detect: meetings.detect } }).catch(() => {});
    }
  }

  // The brain: a provider and its key, nothing else. Its ready setup
  // (profiles.ts) is applied as soon as the key works, and what it runs on
  // this Mac downloads in the background; Settings → Intelligence has the rest.
  let provider = $state<ProfileId | null>(null);
  let applying = $state(false);
  let applied = $state<ProfileId | null>(null);
  const sourceOf = (id: ProfileId): KeySource | null => {
    const i = store.info;
    return (id === "google" ? i?.google_key : id === "openai" ? i?.openai_key : id === "anthropic" ? i?.key : i?.groq_key) ?? null;
  };

  async function pick(id: ProfileId) {
    provider = id;
    error = "";
    react("curious");
    if (keyOf(id)) await useProvider(id);
  }
  async function useProvider(id: ProfileId) {
    applying = true;
    error = "";
    try {
      const voice = await applyProfile(id, await call<VoiceInfo>("voice_info"));
      applied = id;
      react("excited");
      void prepareLocal(id, voice).catch(() => {});
    } catch (e) {
      error = why(e);
    } finally {
      applying = false;
    }
  }
  async function keyed() {
    await store.refreshInfo();
    if (provider && keyOf(provider)) await useProvider(provider);
  }

  // Yoru, the guide: a reaction for each step, and what he says.
  let reaction = $state<{ name: string; at: number } | null>(null);
  const react = (name: string) => (reaction = { name, at: Date.now() });
  const STEP_REACTION = ["wave", "curious", "excited", "thinking", "point", "love"];
  /** A line said instead of the step's, for a moment (the microphone given). */
  let aside = $state<string | null>(null);
  const line = $derived(leaving ? t.ready : (aside ?? t.says[step] ?? ""));
  let typed = $state("");
  $effect(() => {
    const whole = line;
    if (reduced) return void (typed = whole);
    typed = "";
    let n = 0;
    const timer = setInterval(() => {
      n++;
      typed = whole.slice(0, n);
      if (n >= whole.length) clearInterval(timer);
    }, 26);
    return () => clearInterval(timer);
  });
  const talking = $derived(typed.length < line.length);

  function go(to: number) {
    dir = to > step ? 1 : -1;
    aside = null;
    step = to;
    react(STEP_REACTION[to] ?? "wave");
  }
  react("wave");

  function granted(kind: PermissionKind) {
    if (kind !== "microphone") return react("happy");
    aside = t.granted;
    react("excited");
  }

  const next = async () => {
    if (step === 1) await saveMac();
    go(step + 1);
  };
  const back = () => go(step - 1);

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
    await leave();
  }

  async function skip() {
    if (step === 1) await saveMac();
    await leave();
  }

  /** Yoru says goodbye to the tour, the night opens, then the app. */
  async function leave() {
    leaving = true;
    react("dance");
    await new Promise((r) => setTimeout(r, reduced ? 0 : 1500));
    await store.finishOnboarding();
  }

  // The step's way in and out: from the side it comes from, out of focus.
  function enter(_: Element) {
    return {
      duration: reduced ? 0 : 620,
      delay: reduced ? 0 : 140,
      easing: cubicOut,
      css: (k: number, u: number) => `opacity:${k}; transform: translateX(${u * 70 * dir}px) scale(${0.97 + 0.03 * k}); filter: blur(${u * 12}px)`,
    };
  }
  function exit(_: Element) {
    return {
      duration: reduced ? 0 : 280,
      easing: cubicOut,
      css: (k: number, u: number) => `opacity:${k}; transform: translateX(${-u * 50 * dir}px) scale(${1 - 0.03 * u}); filter: blur(${u * 8}px)`,
    };
  }

  // The sky: stars where they fall, each twinkling at its own pace.
  const stars = Array.from({ length: 70 }, () => ({
    x: Math.random() * 100,
    y: Math.random() * 100,
    r: Math.random() < 0.12 ? 2.4 : 1 + Math.random() * 1.2,
    d: 2 + Math.random() * 5,
    o: Math.random() * 6,
  }));
  /** The burst at the end: sparks flying out from Yoru. */
  const sparks = Array.from({ length: 28 }, (_, i) => ({ a: (i / 28) * 360 + Math.random() * 10, d: 120 + Math.random() * 220, s: 3 + Math.random() * 5 }));
  /** The title's letters, each rising after the one before. */
  const letters = $derived([...t.welcome.title]);
</script>

<div class="onboard" class:leaving role="dialog" aria-modal="true" aria-labelledby="setup-title">
  <div class="sky" aria-hidden="true">
    <span class="aurora a1"></span>
    <span class="aurora a2"></span>
    <span class="aurora a3"></span>
    {#each stars as s, i (i)}
      <i class="star" style:left="{s.x}%" style:top="{s.y}%" style:--r="{s.r}px" style:--d="{s.d}s" style:--o="{s.o}s"></i>
    {/each}
  </div>

  <header>
    <div class="progress" role="progressbar" aria-valuemin={1} aria-valuemax={LAST + 1} aria-valuenow={step + 1} aria-label={t.stepOf(step + 1, LAST + 1)}>
      {#each t.steps as s, i (i)}
        <div class="bar" class:on={i === step} class:past={i < step} title={s}><span></span></div>
      {/each}
    </div>
    <span class="where">{t.stepOf(step + 1, LAST + 1)} · {t.steps[step]}</span>
    {#if !store.micMissing}<button class="btn quiet small" onclick={skip}>{t.skip}</button>{/if}
  </header>

  <div class="stage">
    <aside class="guide" class:center={step === 0}>
      <div class="bubble" aria-live="polite">
        <span>{typed}<span class="caret" class:on={talking}></span></span>
        <span class="ghost" aria-hidden="true">{line}</span>
      </div>
      <div class="float">
        <div class="yoru">
          <YoruBoard board="live" size={step === 0 ? 240 : 190} voice={talking ? "speaking" : "idle"} {reaction} follow />
        </div>
        <span class="shadow"></span>
      </div>
      {#if leaving}
        <div class="sparks" aria-hidden="true">
          {#each sparks as p, i (i)}<i style:--a="{p.a}deg" style:--d="{p.d}px" style:--s="{p.s}px"></i>{/each}
        </div>
      {/if}
    </aside>

    <div class="scene">
      {#key step}
        <div class="step" in:enter out:exit>
          {#if step === 0}
            <h1 id="setup-title" class="hero" aria-label={t.welcome.title}>
              {#each letters as c, i (i)}<span style:--i={i}>{c === " " ? " " : c}</span>{/each}
            </h1>
            <p class="lead rise" style:--i={4}>{t.welcome.text}</p>
            <h2 class="label rise" style:--i={5}>{i18n.t.settings.appLanguage}</h2>
            <section class="tiles rise" style:--i={6} aria-label={i18n.t.settings.appLanguage}>
              {#each LANG_CHOICES as l (l ?? "system")}
                <button class="tile" class:on={(i18n.choice ?? null) === l} onclick={() => (i18n.set(l), react("happy"))}>
                  <b>{l ? langName(l) : i18n.t.settings.system}</b>
                  <span>{langName(l ?? i18n.lang)}</span>
                </button>
              {/each}
            </section>
          {:else if step === 1}
            <h1 id="setup-title" class="rise" style:--i={0}>{t.access.title}</h1>
            <p class="lead rise" style:--i={1}>{t.access.text}</p>
            <div class="rise" style:--i={2}><PermissionList hints={t.access.hints} ongrant={granted} /></div>
            <section class="panel rise" style:--i={3}>
              <div class="setting">
                <div class="what"><b>{i18n.t.board.notch}</b><span>{t.access.notchHint}</span></div>
                <div class="how"><Toggle checked={notch} label={i18n.t.board.notch} onchange={(on) => ((notch = on), on && react("happy"))} /></div>
              </div>
              {#if meetings}
                <div class="setting">
                  <div class="what"><b>{i18n.t.meetings.detect}</b><span>{t.access.meetingsHint}</span></div>
                  <div class="how"><Toggle checked={meetings.detect} label={i18n.t.meetings.detect} onchange={(on) => (meetings = { ...meetings!, detect: on })} /></div>
                </div>
              {/if}
            </section>
            {#if store.micMissing}<p class="status rise" style:--i={4}><span class="dot wait"></span>{t.access.waiting}</p>{/if}
          {:else if step === 2}
            <h1 id="setup-title" class="rise" style:--i={0}>{t.board.title}</h1>
            <p class="lead rise" style:--i={1}>{t.board.text}</p>
            <div class="board rise" style:--i={2} class:found={store.boardState === "live"}>
              <div class="disc"><YoruBoard board={store.boardState === "off" ? "off" : "live"} size={120} /></div>
              <div>
                <p class="status"><span class="dot {store.boardState === 'live' ? 'done' : 'think'}"></span>{store.boardState === "live" ? t.board.found : t.board.waiting}</p>
                {#if store.update}<p class="faint">{t.board.update(store.update.to)}</p>{/if}
              </div>
            </div>
          {:else if step === 3}
            <h1 id="setup-title" class="rise" style:--i={0}>{t.brain.title}</h1>
            <p class="lead rise" style:--i={1}>{t.brain.text}</p>
            <div class="providers">
              {#each PROFILES as id, i (id)}
                <button class="tile provider rise" style:--i={2 + i} class:on={provider === id} disabled={applying} onclick={() => pick(id)}>
                  <ServiceIcon id="ai-{id}" name={i18n.t.profiles.names[id]} size={44} />
                  <b>{i18n.t.profiles.names[id]}</b>
                  {#if applied === id}<span class="ready" aria-hidden="true"></span>{/if}
                </button>
              {/each}
            </div>
            {#if provider}
              {#key provider}
                <div class="keyed rise" style:--i={0}>
                  {#if applying}
                    <p class="status"><span class="dot think"></span>{t.brain.applying}</p>
                  {:else if applied === provider}
                    <p class="status"><span class="dot done"></span>{t.brain.ok}</p>
                  {:else}
                    <KeyField provider={provider} source={sourceOf(provider)} onchange={keyed} />
                  {/if}
                </div>
              {/key}
            {/if}
            {#if error && step === 3}<p class="error-text">{error}</p>{/if}
          {:else if step === 4}
            <h1 id="setup-title" class="rise" style:--i={0}>{t.service.title}</h1>
            <p class="lead rise" style:--i={1}>{t.service.text}</p>
            <div class="services">
              {#each quick as p, i (p.id)}
                <button class="tile inset service rise" style:--i={2 + i} class:done={connected.has(p.id)} disabled={!!busy} onclick={() => connect(p.id)}>
                  <ServiceIcon id={p.id} name={p.name} url={p.url} size={30} />
                  <b>{p.name}</b>
                  <span class="what">{busy === p.id ? i18n.t.connectors.signingIn : connected.has(p.id) ? i18n.t.connectors.state.connected : p.notes}</span>
                </button>
              {/each}
            </div>
            {#if busy}<button class="btn quiet small" onclick={() => call("cancel_login", { id: busy })}>{i18n.t.connectors.cancel}</button>{/if}
            {#if error}<p class="error-text">{error}</p>{/if}
          {:else}
            <h1 id="setup-title" class="rise" style:--i={0}>{t.you.title}</h1>
            <p class="lead rise" style:--i={1}>{t.you.text}</p>
            <textarea class="rise" style:--i={2} rows="6" bind:value={about} placeholder={t.you.placeholder}></textarea>
          {/if}
        </div>
      {/key}
    </div>
  </div>

  <footer>
    <span class="spacer"></span>
    {#if step > 0}<button class="btn" onclick={back}>{t.back}</button>{/if}
    {#if step < LAST}
      <button class="btn primary go" disabled={step === 1 && store.micMissing} onclick={next}>{step === 4 && !connected.size ? t.service.later : t.next}</button>
    {:else}
      <button class="btn primary go" disabled={leaving} onclick={finish}>{t.done}</button>
    {/if}
  </footer>
</div>

<style>
  /* Yoru's night whatever the theme: the dark tokens again, here. */
  .onboard {
    --night: #000000;
    --ink: rgba(16, 15, 26, 0.72);
    --ink-2: rgba(26, 24, 40, 0.8);
    --ink-3: rgba(40, 37, 60, 0.8);
    --rim: rgba(236, 233, 245, 0.09);
    --rim-2: rgba(236, 233, 245, 0.14);
    --hover: rgba(236, 233, 245, 0.07);
    --pearl: #ece9f5;
    --pearl-mid: #c3bedb;
    --pearl-deep: #6e6890;
    --haze: #a7a2c2;
    --dusk: #6b6687;
    --eye: #17151f;
    --think: #4aa3ff;
    --done: #40e080;
    --wait: #ff9f43;
    --wrong: #ff5a5a;
    --field: rgba(26, 24, 40, 0.8);
    --shadow: 0 30px 80px -20px rgba(0, 0, 0, 0.8);
    --spring: cubic-bezier(0.34, 1.56, 0.64, 1);
    --glide: cubic-bezier(0.2, 0.8, 0.2, 1);
    color-scheme: dark;
    position: fixed;
    inset: 0;
    z-index: 30;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    color: var(--pearl);
    background: radial-gradient(120% 90% at 50% 40%, #1d1a33 0%, #0c0b16 55%, #040407 100%);
    /* Clip, not hidden: a focused button must not scroll the night away. */
    overflow: clip;
    animation: dawn 0.9s var(--glide) both;
  }
  .onboard.leaving {
    animation: dusk-out 1.1s 0.45s cubic-bezier(0.7, 0, 0.84, 0) forwards;
  }
  @keyframes dawn {
    from {
      opacity: 0;
    }
  }
  @keyframes dusk-out {
    to {
      opacity: 0;
      transform: scale(1.08);
      filter: blur(8px);
    }
  }

  /* The sky: three slow lights and the stars. */
  .sky {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .aurora {
    position: absolute;
    border-radius: 50%;
    filter: blur(80px);
    opacity: 0.55;
    mix-blend-mode: screen;
  }
  .a1 {
    width: 620px;
    height: 620px;
    left: -140px;
    top: 10%;
    background: radial-gradient(circle, #6a5cff, transparent 65%);
    animation: drift1 22s ease-in-out infinite alternate;
  }
  .a2 {
    width: 520px;
    height: 520px;
    right: -120px;
    top: -160px;
    background: radial-gradient(circle, #3fa8ff, transparent 65%);
    opacity: 0.35;
    animation: drift2 26s ease-in-out infinite alternate;
  }
  .a3 {
    width: 560px;
    height: 560px;
    right: 10%;
    bottom: -280px;
    background: radial-gradient(circle, #c06cff, transparent 65%);
    opacity: 0.3;
    animation: drift3 30s ease-in-out infinite alternate;
  }
  @keyframes drift1 {
    to {
      transform: translate(140px, -60px) scale(1.15);
    }
  }
  @keyframes drift2 {
    to {
      transform: translate(-160px, 120px) scale(0.9);
    }
  }
  @keyframes drift3 {
    to {
      transform: translate(-120px, -90px) scale(1.2);
    }
  }
  .star {
    position: absolute;
    width: var(--r);
    height: var(--r);
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 0 6px rgba(255, 255, 255, 0.7);
    animation: twinkle var(--d) var(--o) ease-in-out infinite;
    opacity: 0.2;
  }
  @keyframes twinkle {
    50% {
      opacity: 0.85;
      transform: scale(1.3);
    }
  }

  header {
    position: relative;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 26px 40px 0;
    /* The Mac's buttons sit at the top left. */
    padding-left: 96px;
  }
  .progress {
    display: flex;
    gap: 6px;
    flex: 0 1 360px;
  }
  .bar {
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: rgba(236, 233, 245, 0.12);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    width: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #c3bedb, #fff);
    transform: scaleX(0);
    transform-origin: left;
    transition: transform 0.7s var(--glide);
  }
  .bar.past span,
  .bar.on span {
    transform: scaleX(1);
  }
  .bar.on span {
    box-shadow: 0 0 12px rgba(255, 255, 255, 0.6);
  }
  .where {
    flex: 1;
    color: var(--haze);
    font-size: var(--fs-meta);
  }

  .stage {
    position: relative;
    display: grid;
    grid-template-columns: minmax(260px, 0.8fr) minmax(0, 1.4fr);
    grid-template-rows: minmax(0, 1fr);
    align-items: center;
    gap: 24px;
    min-height: 0;
    padding: 0 56px;
  }

  /* Yoru, the guide: floating, with what he says above him. */
  .guide {
    position: relative;
    display: grid;
    justify-items: center;
    gap: 6px;
    transition: transform 0.9s var(--spring);
  }
  .guide.center {
    transform: translateY(10px) scale(1.04);
  }
  .float {
    position: relative;
    display: grid;
    justify-items: center;
    animation: bob 4.5s ease-in-out infinite;
  }
  .yoru {
    display: grid;
    filter: drop-shadow(0 20px 40px rgba(106, 92, 255, 0.35));
    transition: transform 0.9s var(--spring);
  }
  .yoru :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .shadow {
    width: 110px;
    height: 16px;
    margin-top: -18px;
    border-radius: 50%;
    background: radial-gradient(closest-side, rgba(0, 0, 0, 0.65), transparent);
    animation: squash 4.5s ease-in-out infinite;
  }
  @keyframes bob {
    50% {
      transform: translateY(-14px);
    }
  }
  @keyframes squash {
    50% {
      transform: scaleX(0.78);
      opacity: 0.6;
    }
  }
  .leaving .yoru {
    transform: scale(1.12);
  }

  .bubble {
    position: relative;
    display: grid;
    max-width: 280px;
    padding: 12px 16px;
    border-radius: 18px;
    background: rgba(236, 233, 245, 0.95);
    color: #17151f;
    font: 550 var(--fs-body)/1.4 var(--sans);
    box-shadow: 0 14px 40px -12px rgba(0, 0, 0, 0.6);
    animation: bubble-in 0.6s 0.3s var(--spring) both;
  }
  /* The whole line, hidden, holds the bubble's size while it types. */
  .bubble > span {
    grid-area: 1 / 1;
  }
  .ghost {
    visibility: hidden;
  }
  .bubble::after {
    content: "";
    position: absolute;
    left: 50%;
    bottom: -7px;
    width: 14px;
    height: 14px;
    background: inherit;
    transform: translateX(-50%) rotate(45deg);
    border-radius: 3px;
  }
  .caret {
    display: inline-block;
    width: 2px;
    height: 1em;
    margin-left: 2px;
    vertical-align: -2px;
    background: currentColor;
    opacity: 0;
  }
  .caret.on {
    animation: blink 0.8s steps(1) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 1;
    }
  }
  @keyframes bubble-in {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.85);
    }
  }

  /* The burst at the end. */
  .sparks {
    position: absolute;
    left: 50%;
    top: 58%;
  }
  .sparks i {
    position: absolute;
    width: var(--s);
    height: var(--s);
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 0 10px #b9b0ff;
    animation: spark 1.2s cubic-bezier(0.1, 0.8, 0.3, 1) forwards;
  }
  @keyframes spark {
    from {
      transform: rotate(var(--a)) translateX(0) scale(1);
      opacity: 1;
    }
    to {
      transform: rotate(var(--a)) translateX(var(--d)) scale(0);
      opacity: 0;
    }
  }

  /* The step: stacked, so the one leaving and the one coming overlap. */
  .scene {
    display: grid;
    align-items: center;
    max-height: 100%;
    min-height: 0;
    overflow: hidden auto;
    /* Room for the glows and shadows inside the scroller, which clips
       them: the padding is taken back with a margin, and the edges fade. */
    padding: 40px 40px;
    margin: 0 -40px;
    mask-image: linear-gradient(transparent, #000 32px, #000 calc(100% - 32px), transparent);
    scrollbar-width: none;
  }
  .step {
    grid-area: 1 / 1;
    display: grid;
    gap: 14px;
    align-content: start;
    max-width: 640px;
  }
  h1 {
    font: 700 34px/1.12 var(--display);
    letter-spacing: -0.02em;
    text-wrap: balance;
  }
  .hero {
    font-size: 52px;
    background: linear-gradient(100deg, #fff 20%, #c9c1ff 50%, #fff 80%);
    background-size: 200% 100%;
    background-clip: text;
    -webkit-background-clip: text;
    animation: shine 6s 1.4s linear infinite;
  }
  .hero span {
    display: inline-block;
    color: transparent;
    background: inherit;
    background-clip: text;
    -webkit-background-clip: text;
    -webkit-text-fill-color: #fff;
    animation: letter 0.7s calc(0.3s + var(--i) * 0.045s) var(--spring) both;
  }
  @keyframes letter {
    from {
      opacity: 0;
      transform: translateY(26px) rotate(8deg);
      filter: blur(6px);
    }
  }
  @keyframes shine {
    to {
      background-position: -200% 0;
    }
  }
  .lead {
    color: var(--haze);
    font-size: 16px;
    line-height: 1.5;
    max-width: 560px;
  }
  .label {
    margin-top: 10px;
    font: 600 var(--fs-body)/1.2 var(--sans);
  }
  /* Each part rises after the one before. */
  .rise {
    animation: rise 0.7s calc(0.18s + var(--i, 0) * 0.07s) var(--glide) both;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(18px);
      filter: blur(4px);
    }
  }
  .step :global(.panel) {
    backdrop-filter: blur(14px);
  }
  .tile {
    backdrop-filter: blur(14px);
    transition:
      transform 0.35s var(--spring),
      border-color 0.2s,
      background 0.2s;
  }
  .tile:hover:not(:disabled) {
    transform: translateY(-3px);
  }
  .tile.on {
    box-shadow: 0 0 0 1px rgba(236, 233, 245, 0.5), 0 10px 30px -10px rgba(185, 176, 255, 0.6);
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .board {
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 16px 20px;
    border-radius: var(--r-lg);
    border: 1px solid var(--rim);
    background: var(--ink);
    backdrop-filter: blur(14px);
    transition: border-color 0.5s;
  }
  .board.found {
    border-color: color-mix(in srgb, var(--done) 45%, transparent);
    box-shadow: 0 0 40px -14px var(--done);
  }
  .disc {
    display: grid;
    padding: 6px;
    border-radius: 50%;
    background: #000;
    box-shadow: 0 0 0 6px #1a1826, 0 18px 40px -12px rgba(0, 0, 0, 0.9);
  }
  .providers {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
  }
  .provider {
    position: relative;
    justify-items: center;
    gap: 10px;
    padding: 18px 10px 14px;
  }
  /* The one in use: a green tick on its corner. */
  .ready {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--done);
    box-shadow: 0 0 10px var(--done);
    animation: tick-pop 0.5s var(--spring) both;
  }
  @keyframes tick-pop {
    from {
      transform: scale(0);
    }
  }
  .keyed {
    padding: 14px 16px;
    border-radius: var(--r-lg);
    border: 1px solid var(--rim);
    background: var(--ink);
    backdrop-filter: blur(14px);
  }
  .services {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .service {
    gap: 4px;
    grid-template-columns: auto minmax(0, 1fr);
    column-gap: 12px;
    align-items: center;
  }
  .service .what {
    grid-column: 2;
  }
  .service.done {
    border-color: color-mix(in srgb, var(--done) 45%, transparent);
  }
  textarea {
    background: var(--field);
    backdrop-filter: blur(14px);
  }

  footer {
    position: relative;
    display: flex;
    gap: 10px;
    padding: 0 56px 32px;
  }
  .spacer {
    flex: 1;
  }
  footer .btn {
    height: 40px;
    padding: 0 22px;
  }
  /* Next glows and a light runs over it, to say where to go. */
  .go {
    position: relative;
    overflow: hidden;
    min-width: 120px;
    box-shadow: 0 8px 30px -8px rgba(185, 176, 255, 0.7);
    transition:
      transform 0.3s var(--spring),
      box-shadow 0.3s;
  }
  .go:hover:not(:disabled) {
    transform: translateY(-2px) scale(1.03);
    box-shadow: 0 12px 36px -8px rgba(185, 176, 255, 0.9);
  }
  .go::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 30%, rgba(255, 255, 255, 0.8) 50%, transparent 70%);
    transform: translateX(-120%);
    animation: sweep 3.2s 1.2s ease-in-out infinite;
  }
  .go:disabled::after {
    display: none;
  }
  @keyframes sweep {
    40%,
    100% {
      transform: translateX(120%);
    }
  }

  @media (max-width: 900px) {
    .stage {
      grid-template-columns: 1fr;
      padding: 0 28px;
    }
    .guide {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .onboard *,
    .onboard {
      animation-duration: 0.01s !important;
      animation-iteration-count: 1 !important;
      transition-duration: 0.01s !important;
    }
  }
</style>
