<!--
  Settings as Yoru's wiring: the mirror in the middle, and each part of the
  settings a card wired to him by a trace, with what it says right now. On
  the left what Yoru is (his brain, his voice, the language he answers
  in, what he does with a meeting); on the right what he's connected to
  (the services' keys, the Mac, the board, the Pocket, the data folder). A card opens its own page, with the
  everyday choices first and the technical ones under "Advanced".
-->
<script lang="ts">
  import { look, THEMES } from "../look.svelte";
  import { fly } from "svelte/transition";
  import { call, why } from "../api";
  import { i18n, LANGUAGES, type Lang } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { AssistantInfo, AssistantSettings, Brain, Effort, Gaze, KeySource, McpInfo, MeetingSettings, VoiceInfo, VoiceSettings } from "../types";
  import YoruBoard from "../YoruBoard.svelte";
  import Board from "../ui/Board.svelte";
  import KeyField from "../ui/KeyField.svelte";
  import PermissionList from "../ui/PermissionList.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import Voice from "../ui/Voice.svelte";
  import MeetingSettingsPage from "../ui/MeetingSettings.svelte";
  import PocketSettingsPage from "../ui/PocketSettings.svelte";
  import Profiles from "../ui/Profiles.svelte";
  import { currentProfile } from "../profiles";
  import { PAGE_ICONS, SETTINGS_PAGES } from "../icons/pages";
  import Icon from "../ui/Icon.svelte";
  import type { IconName } from "../icons/lucide";

  const t = $derived(i18n.t.settings);
  const s = $derived(store.info?.settings);
  const EFFORTS: Effort[] = ["low", "medium", "high", "xhigh", "max"];
  /** Claude's models, by what they're like; the ids are in the option's value. */
  const CLAUDE_MODELS = $derived([
    ["claude-opus-5-5", t.claudeModels.opus],
    ["claude-sonnet-5-5", t.claudeModels.sonnet],
    ["claude-haiku-5-5", t.claudeModels.haiku],
  ] as const);
  const brain = $derived(store.info?.brain ?? null);
  const BRAIN_NAMES: Record<Brain, string> = { anthropic: "Claude", openai: "OpenAI", google: "Gemini", groq: "Groq", ollama: "Ollama" };
  const BRAINS: Brain[] = ["anthropic", "openai", "google", "groq", "ollama"];
  const LANG_CHOICES: (Lang | null)[] = [null, ...LANGUAGES.map((l) => l.id)];
  const langName = (id: Lang) => LANGUAGES.find((l) => l.id === id)?.name ?? id;

  // Each brain's model, where it's kept, and the models it offers.
  const MODEL_FIELD = { anthropic: "model", openai: "openai_model", google: "google_model", groq: "groq_model", ollama: "ollama_model" } as const;
  const modelOf = (b: Brain) => (s ? s[MODEL_FIELD[b]] : "");
  const setModel = (b: Brain, v: string) => store.saveSettings((x: AssistantSettings) => (x[MODEL_FIELD[b]] = v));
  const catalog = (b: Brain) => (b === "openai" ? store.info?.openai_models : b === "google" ? store.info?.google_models : b === "groq" ? store.info?.groq_models : undefined) ?? [];

  let voice = $state<VoiceInfo | null>(null);
  let mcp = $state<McpInfo | null>(null);
  let notch = $state(false);
  let meetings = $state<{ settings: MeetingSettings; model: boolean } | null>(null);
  let error = $state("");

  async function readAll() {
    [voice, mcp] = await Promise.all([call<VoiceInfo>("voice_info").catch(() => null), call<McpInfo>("mcp_info").catch(() => null)]);
    const [ms, model] = await Promise.all([call<MeetingSettings>("meeting_settings").catch(() => null), call<boolean>("meeting_model").catch(() => true)]);
    meetings = ms ? { settings: ms, model } : null;
    notch = (await call<{ notch?: boolean }>("status").catch(() => null))?.notch ?? false;
  }
  $effect(() => {
    void readAll();
  });

  // Every key in one place: the brains' (OpenAI's and Groq's serve speech to
  // text too, Google's Gemini Live), the web's, and the voice-only services'.
  type Key = { id: string; name: string; hint: string; source: KeySource | null; placeholder?: string; group: "brains" | "voices" | "web" };
  const keys = $derived.by<Key[]>(() => {
    const i: AssistantInfo | null = store.info;
    const brainKeys: Key[] = [
      { id: "anthropic", name: t.anthropic, hint: t.anthropicHint, source: i?.key ?? null, placeholder: "sk-ant-…", group: "brains" },
      { id: "openai", name: t.openai, hint: t.openaiHint, source: i?.openai_key ?? null, placeholder: "sk-proj-…", group: "brains" },
      { id: "google", name: t.google, hint: t.googleHint, source: i?.google_key ?? null, placeholder: "AIza…", group: "brains" },
      { id: "groq", name: t.groq, hint: t.groqHint, source: i?.groq_key ?? null, placeholder: "gsk_…", group: "brains" },
    ];
    const voices: Key[] = (voice?.providers ?? [])
      .filter((p) => !(BRAINS as string[]).includes(p.id) && p.id !== "tavily")
      .map((p) => ({ id: p.id, name: p.name, hint: i18n.t.board.serviceNotes[p.id] ?? p.note, source: p.key, group: "voices" }));
    return [...brainKeys, ...voices, { id: "tavily", name: t.tavily, hint: t.tavilyHint, source: i?.tavily_key ?? null, placeholder: "tvly-…", group: "web" }];
  });
  const keyOf = (b: Brain) => keys.find((k) => k.id === b);

  // Ollama's models are the ones pulled on this computer: asked when it is the brain.
  let ollamaModels = $state<string[] | null>(null);
  let ollamaError = $state("");
  async function readOllama() {
    ollamaError = "";
    try {
      ollamaModels = await call<string[]>("ollama_models");
    } catch (e) {
      ollamaModels = null;
      ollamaError = why(e);
    }
  }
  $effect(() => {
    if (brain === "ollama") void readOllama();
  });

  async function run<T>(job: () => Promise<T>, then: (v: T) => void) {
    error = "";
    try {
      then(await job());
    } catch (e) {
      error = why(e);
    }
  }
  const registerMcp = (connect: boolean) => run(() => call<McpInfo>("claude_mcp", { connect }), (v) => (mcp = v));
  const setVoice = (change: Partial<VoiceSettings>) =>
    voice && run(() => call<VoiceInfo>("set_voice", { settings: { ...$state.snapshot(voice!.settings), ...change } }), (v) => (voice = v));

  async function setupAgain() {
    await call("set_onboarding", { done: false, tourVersion: null });
    store.onboarding = { done: false, tour_version: store.onboarding?.tour_version ?? 0 };
  }

  // The parts, wired to Yoru: four on the left, five on the right, top to bottom.
  type Part = "brain" | "voice" | "language" | "meetings" | "keys" | "mac" | "board" | "pocket" | "about";
  type Wire = { id: Part; side: -1 | 1; color: string; icon: IconName };
  const WIRES: Wire[] = [
    { id: "brain", side: -1, color: "var(--think)", icon: "sparkles" },
    { id: "voice", side: -1, color: "#c58cff", icon: "audio-waveform" },
    { id: "language", side: -1, color: "#5fd4c8", icon: "languages" },
    { id: "meetings", side: -1, color: "#ff9f6b", icon: "users" },
    { id: "keys", side: 1, color: "var(--wait)", icon: "key-round" },
    { id: "mac", side: 1, color: "#7fb2ff", icon: "laptop" },
    { id: "board", side: 1, color: "var(--done)", icon: "cpu" },
    { id: "pocket", side: 1, color: "#e0559a", icon: "mic" },
    { id: "about", side: 1, color: "var(--pearl-mid)", icon: "info" },
  ];
  const PAGES: Part[] = WIRES.map((w) => w.id);

  let open = $state<Part | null>(null);
  let hover = $state<Part | null>(null);
  const wire = (id: Part) => WIRES.find((w) => w.id === id)!;

  function pick(id: Part, e: Event) {
    hover = null;
    open = id;
    (e.currentTarget as HTMLElement).closest("main")?.scrollTo({ top: 0 });
  }

  const profile = $derived(currentProfile(brain, voice));
  // What each card says now, and its dot.
  const brainModel = $derived(brain ? modelOf(brain) : "");
  const keyed = $derived(keys.filter((k) => k.source).length);
  const firmware = $derived(store.firmware?.state === "version" ? store.firmware.version : null);
  const via = $derived(store.port?.startsWith("wifi:") ? i18n.t.link.wifi : store.port ? i18n.t.link.usb : "");
  const vs = $derived(voice?.settings);
  const pocketSummary = $derived.by(() => {
    const p = store.pocket;
    const tp = i18n.t.pocket;
    if (!p) return { text: tp.unavailable, dot: "" };
    if (p.phase === "off") return { text: tp.phases.off, dot: "" };
    const dot = p.phase === "connected" ? "done" : p.phase === "refused" || p.phase === "no_bluetooth" ? "wrong" : p.phase === "unpaired" ? "" : "think";
    return { text: [tp.phases[p.phase], p.phase === "connected" && p.battery !== null ? `${p.battery}%` : ""].filter(Boolean).join(" · "), dot };
  });
  const summary = $derived<Record<Part, { text: string; dot: string; warn?: boolean }>>({
    brain: !store.info?.ready
      ? { text: brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain, dot: "wrong", warn: true }
      : { text: profile ? i18n.t.profiles.names[profile] : `${t.customShort} · ${brain ? BRAIN_NAMES[brain] : ""}${brainModel ? " · " + brainModel : ""}`, dot: store.info?.limited ? "wait" : "done" },
    voice: {
      text: !vs ? "—" : [vs.enabled ? t.listening : t.notListening, vs.live ? i18n.t.board.live : vs.speak ? "" : t.silent].filter(Boolean).join(" · "),
      dot: vs?.enabled ? "done" : "",
    },
    language: { text: i18n.choice ? langName(i18n.choice) : `${t.system} · ${langName(i18n.lang)}`, dot: "" },
    keys: { text: keyed ? t.keysOf(keyed, keys.length) : t.noKeys, dot: "" },
    mac: { text: [notch ? t.notchOn : t.notchOff, mcp?.registered ? t.claudeCode : ""].filter(Boolean).join(" · "), dot: notch || mcp?.registered ? "done" : "" },
    board:
      store.boardState === "off"
        ? { text: t.boardOff, dot: "" }
        : { text: [store.boardState === "boot" ? i18n.t.link.boot : i18n.t.link.live, via, firmware].filter(Boolean).join(" · "), dot: store.update ? "wait" : store.boardState === "boot" ? "think" : "done" },
    meetings: !meetings
      ? { text: "—", dot: "" }
      : !meetings.model
        ? { text: t.meetingsNoModel, dot: "wait" }
        : { text: `${t.meetingsWhen[meetings.settings.transcribe]} · ${t.meetingsBy(meetings.settings.summary_by.split(":").pop() ?? "")}`, dot: meetings.settings.detect ? "done" : "" },
    pocket: pocketSummary,
    about: { text: t.aboutSummary, dot: "" },
  });

  // The cards sit in two columns either side of Yoru, in the page's flow,
  // so they never overlap; the traces are drawn after, from where the
  // cards and Yoru really are. Too narrow for that, the cards go in a grid
  // under Yoru, without traces.
  /** Between two rows' traces where they meet the case. */
  const ANGLE_STEP = (30 * Math.PI) / 180;
  /** The narrowest stage with Yoru between the columns. */
  const WIRED_MIN = 640;
  /** A card's place on its side, from the middle: −1.5 to 1.5 for four, −1 to 1 for three. */
  function offset(w: Wire) {
    const side = WIRES.filter((x) => x.side === w.side);
    return side.indexOf(w) - (side.length - 1) / 2;
  }
  let stage: HTMLElement | undefined = $state();
  let core: HTMLElement | undefined = $state();
  let stageW = $state(840);
  const wired = $derived(stageW >= WIRED_MIN);
  const yoruSize = $derived(wired ? Math.max(110, Math.min(170, stageW * 0.18)) : 120);
  type Geo = { w: number; h: number; cx: number; cy: number; r: number; cards: Partial<Record<Part, { x: number; y: number }>> };
  let geo = $state<Geo | null>(null);
  function measure() {
    if (!stage || !core) return;
    const box = stage.getBoundingClientRect();
    const c = core.getBoundingClientRect();
    const cards: Geo["cards"] = {};
    for (const el of stage.querySelectorAll<HTMLElement>(".part")) {
      const r = el.getBoundingClientRect();
      const w = wire(el.dataset.part as Part);
      cards[w.id] = { x: (w.side < 0 ? r.right : r.left) - box.left, y: r.top + r.height / 2 - box.top };
    }
    geo = { w: box.width, h: box.height, cx: c.left + c.width / 2 - box.left, cy: c.top + c.height / 2 - box.top, r: (c.width / 2) * 0.92, cards };
  }
  $effect(() => {
    if (!stage) return;
    void wired;
    const watch = new ResizeObserver(() => measure());
    watch.observe(stage);
    for (const el of stage.querySelectorAll(".part")) watch.observe(el);
    measure();
    return () => watch.disconnect();
  });
  function trace(w: Wire, g: Geo) {
    const card = g.cards[w.id];
    if (!card) return null;
    const { x: ax, y: ay } = card;
    /** Where the trace meets the case, from straight up (−90°) to straight down. */
    const a = offset(w) * ANGLE_STEP;
    const ex = g.cx + w.side * g.r * Math.cos(a);
    const ey = g.cy + g.r * Math.sin(a);
    // Out of the card flat, then in at 45°, as copper runs on a board.
    const bx = w.side < 0 ? Math.max(ax, ex - Math.abs(ey - ay)) : Math.min(ax, ex + Math.abs(ey - ay));
    return { d: `M ${ax} ${ay} H ${bx} L ${ex} ${ey}`, ax, ay, ex, ey };
  }
  // The pages that aren't part of Yoru's wiring, opened from the menu under it.
  const MORE_COLORS: Record<(typeof SETTINGS_PAGES)[number], string> = { tasks: "var(--wait)", routines: "#7fb2ff", connectors: "var(--think)", spoken: "#c58cff", animations: "var(--done)" };
  const moreDot = (p: (typeof SETTINGS_PAGES)[number]) =>
    p === "connectors" && store.connectors.some((c) => c.enabled && (c.state === "needs_login" || c.state === "error")) ? "wait" : "";

  // Yoru looks at the card under the pointer.
  const gaze = $derived<Gaze | null>(hover ? { x: wire(hover).side * 0.9, y: offset(wire(hover)) * 0.5, movedAt: store.now } : store.gaze);
</script>

<div class="page">
  {#if open}
    {@const w = wire(open)}
    <div class="sub" style:--c={w.color} in:fly={{ y: 10, duration: 260 }}>
      <button class="btn quiet small back" onclick={() => ((open = null), readAll())}>
        <Icon name="chevron-left" size={16} />{t.back}
      </button>
      <div class="sub-head">
        <span class="glyph big"><Icon name={w.icon} size={28} /></span>
        <div>
          <h1>{t.parts[open]}</h1>
          <p class="lede">{t.partLedes[open]}</p>
        </div>
      </div>
      <nav class="hop" aria-label={t.title}>
        {#each PAGES as p (p)}
          <button class="chip small tint" class:on={open === p} style:--c={wire(p).color} onclick={(e) => pick(p, e)}>{t.parts[p]}</button>
        {/each}
      </nav>
      {#if open === "brain"}{@render brainPage()}
      {:else if open === "voice"}<Voice />
      {:else if open === "language"}{@render languagePage()}
      {:else if open === "keys"}{@render keysPage()}
      {:else if open === "mac"}{@render macPage()}
      {:else if open === "board"}<Board />
      {:else if open === "meetings"}<MeetingSettingsPage />
      {:else if open === "pocket"}<PocketSettingsPage />
      {:else}{@render aboutPage()}{/if}
      {#if error}<p class="error-text note">{error}</p>{/if}
    </div>
  {:else}
    <div class="page-head">
      <div>
        <h1>{t.title}</h1>
        <p class="lede">{t.lede}</p>
      </div>
    </div>

    <div class="stage" class:wired bind:this={stage} bind:clientWidth={stageW}>
      {#if wired && geo}
        <svg class="wires" viewBox="0 0 {geo.w} {geo.h}" aria-hidden="true">
          {#each WIRES as w, i (w.id)}
            {@const p = trace(w, geo)}
            {#if p}
              <g class="wire" class:lit={hover === w.id} class:warn={summary[w.id].warn} style:--c={w.color} style:--i={i}>
                <path class="copper" d={p.d} pathLength="1" />
                <path class="pulse" d={p.d} pathLength="100" />
                <circle class="pad" cx={p.ax} cy={p.ay} r="4.5" />
                <circle class="pad" cx={p.ex} cy={p.ey} r="3.5" />
              </g>
            {/if}
          {/each}
        </svg>
      {/if}

      <div class="yoru" bind:this={core}>
        <YoruBoard board={store.boardState === "off" ? "off" : "live"} size={yoruSize} voice={store.voice} {gaze} />
      </div>

      {#each [-1, 1] as side (side)}
        <div class="col" class:left={side < 0}>
          {#each WIRES.filter((w) => w.side === side) as w (w.id)}
            {@const sum = summary[w.id]}
            {@const i = WIRES.indexOf(w)}
            <button
              class="tile part"
              class:warn={sum.warn}
              data-part={w.id}
              style:--c={w.color}
              style:--i={i}
              onmouseenter={() => (hover = w.id)}
              onmouseleave={() => (hover = null)}
              onfocus={() => (hover = w.id)}
              onblur={() => (hover = null)}
              onclick={(e) => pick(w.id, e)}
            >
              <span class="glyph"><Icon name={w.icon} size={20} /></span>
              <span class="words">
                <b>{t.parts[w.id]}{#if sum.dot}<span class="dot {sum.dot}"></span>{/if}</b>
                <span class="says" class:error-text={sum.warn} title={sum.text}>{sum.text}</span>
              </span>
              <span class="chev"><Icon name="chevron-right" size={16} /></span>
            </button>
          {/each}
        </div>
      {/each}
    </div>

    <h2 class="section-title more-title">{t.more}</h2>
    <nav class="menu panel" aria-label={t.more}>
      {#each SETTINGS_PAGES as p (p)}
        {@const dot = moreDot(p)}
        <button style:--c={MORE_COLORS[p]} onclick={() => store.go(p)}>
          <span class="glyph"><Icon name={PAGE_ICONS[p]} size={20} /></span>
          <span class="words">
            <b>{i18n.t.nav[p]}{#if dot}<span class="dot {dot}"></span>{/if}</b>
            <span class="says">{t.moreNotes[p]}</span>
          </span>
          <span class="chev"><Icon name="chevron-right" size={16} /></span>
        </button>
      {/each}
    </nav>

    <h2 class="section-title more-title">{t.look}</h2>
    <div class="panel setting looks">
      <div class="what"><b>{t.theme}</b><span>{t.lookNote}</span></div>
      <div class="how seg" role="radiogroup" aria-label={t.look}>
        {#each THEMES as th (th)}
          <button role="radio" aria-checked={look.theme === th} class:on={look.theme === th} onclick={() => look.setTheme(th)}>{t.themes[th]}</button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<!-- A setup first (Profiles); its parts, the brain and its model, under "Custom". How hard Claude thinks and whether it searches the web are advanced. -->
{#snippet brainPage()}
  {#if s}
    <Profiles onchange={readAll} />
    <details class="more" open={!profile}>
      <summary class="section-title">{t.customize}</summary>
      <p class="dim small">{t.customizeHint}</p>
      <section class="tiles" aria-label={t.provider}>
        {#each BRAINS as b (b)}
          <button class="tile" class:on={brain === b} onclick={() => store.saveSettings((x) => (x.provider = b))}>
            <b>{BRAIN_NAMES[b]}</b>
            <span>{t.brainNotes[b]}</span>
            {#if brain === b && store.info?.ready}<span class="tag">{t.inUse}</span>{/if}
          </button>
        {/each}
      </section>
      {#if !store.info?.ready}
        <p class="error-text note">{brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain}</p>
      {:else if store.info?.limited}
        <p class="error-text note">{store.info.limited}</p>
      {/if}
      {#if brain}
        <section class="panel active">
          {#if brain !== "ollama"}
            {@const k = keyOf(brain)}
            {#if k}
              <div class="block">
                <div class="what"><b>{t.keyOf(k.name)}</b><span class="dim">{k.hint}</span></div>
                <KeyField provider={k.id} source={k.source} placeholder={k.placeholder} onchange={() => (store.refreshInfo(), readAll())} />
              </div>
            {/if}
          {/if}
          {@render modelRow(brain)}
          {#if brain === "anthropic"}
            <div class="setting">
              <div class="what"><b>{t.effort}</b><span class="dim">{t.effortHint}</span></div>
              <div class="how">
                <select value={s.effort} onchange={(e) => store.saveSettings((x) => (x.effort = e.currentTarget.value as Effort))} aria-label={t.effort}>
                  {#each EFFORTS as e (e)}<option value={e}>{t.efforts[e]}</option>{/each}
                </select>
              </div>
            </div>
            <div class="setting">
              <div class="what"><b>{t.webSearch}</b></div>
              <div class="how"><Toggle checked={s.web_search} label={t.webSearch} onchange={(on) => store.saveSettings((x) => (x.web_search = on))} /></div>
            </div>
          {:else}
            <p class="dim small">{t.groqNote}</p>
          {/if}
        </section>
      {/if}
    </details>
  {/if}
{/snippet}

<!-- A brain's model: Claude's typed or picked, Ollama's from what's pulled here, the others' from their catalog. -->
{#snippet modelRow(b: Brain, name = t.model)}
  {#if s}
    {#if b === "ollama"}
      <div class="setting">
        <div class="what">
          <b>{name}</b><span class="dim">{t.ollamaHint}</span>
          {#if ollamaError}<span class="error-text">{ollamaError}</span>{:else if ollamaModels && !ollamaModels.length}<span class="error-text">{t.ollamaNone}</span>{/if}
        </div>
        <div class="how">
          <input class="model mono" type="text" list="ollama-models" value={s.ollama_model} placeholder={t.ollamaPick} spellcheck="false" onchange={(e) => setModel("ollama", e.currentTarget.value.trim())} aria-label={name} />
          <datalist id="ollama-models">{#each ollamaModels ?? [] as m (m)}<option value={m}></option>{/each}</datalist>
          <button class="btn small" onclick={readOllama}>{t.ollamaRefresh}</button>
          <button class="btn small" onclick={() => call("open_key_page", { provider: "ollama" })}>{t.ollamaGet}</button>
        </div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.ollamaThink}</b><span class="dim">{t.ollamaThinkHint}</span></div>
        <div class="how"><Toggle checked={s.think} label={t.ollamaThink} onchange={(on) => store.saveSettings((x) => (x.think = on))} /></div>
      </div>
    {:else}
      <div class="setting">
        <div class="what"><b>{name}</b></div>
        <div class="how">
          {#if b === "anthropic"}
            <select value={s.model} onchange={(e) => setModel("anthropic", e.currentTarget.value)} aria-label={name}>
              {#each CLAUDE_MODELS as [m, label] (m)}<option value={m}>{label}</option>{/each}
              {#if !CLAUDE_MODELS.some(([m]) => m === s.model)}<option value={s.model}>{s.model}</option>{/if}
            </select>
          {:else}
            <select value={modelOf(b)} onchange={(e) => setModel(b, e.currentTarget.value)} aria-label={name}>
              {#each catalog(b) as m (m.id)}<option value={m.id}>{m.id} — {m.note}</option>{/each}
            </select>
          {/if}
        </div>
      </div>
    {/if}
  {/if}
{/snippet}

{#snippet languagePage()}
  <section class="tiles langs" aria-label={t.appLanguage}>
    {#each LANG_CHOICES as l (l ?? "system")}
      <button class="tile" class:on={(i18n.choice ?? null) === l} onclick={() => i18n.set(l)}>
        <b>{l ? langName(l) : t.system}</b>
        <span>{langName(l ?? i18n.lang)}</span>
      </button>
    {/each}
  </section>
{/snippet}

<!-- Every service's key, grouped by what it's for. -->
{#snippet keysPage()}
  <p class="dim small">{t.keysPrivacy}</p>
  {#each ["brains", "voices", "web"] as g (g)}
    {@const list = keys.filter((k) => k.group === g)}
    {#if list.length}
      <h2 class="section-title">{t.keyGroups[g]}</h2>
      <section class="panel">
        {#each list as k (k.id)}
          <div class="block">
            <div class="what"><b>{k.name}</b><span class="dim">{k.hint}</span></div>
            <KeyField provider={k.id} source={k.source} placeholder={k.placeholder} onchange={() => (store.refreshInfo(), readAll())} />
          </div>
        {/each}
      </section>
    {/if}
  {/each}
{/snippet}

<!-- Yoru on this Mac: in the notch, and in Claude Code through his MCP server. -->
{#snippet macPage()}
  {@const tb = i18n.t.board}
  <h2 class="section-title">{i18n.t.permissions.title}</h2>
  <PermissionList manage />
  <p class="dim small perm-hint">{i18n.t.permissions.settingsHint}</p>
  <section class="panel">
    <div class="setting">
      <div class="what"><b>{tb.notch}</b><span>{tb.notchHint}</span></div>
      <div class="how"><Toggle checked={notch} label={tb.notch} onchange={(on) => ((notch = on), call("set_notch", { on }))} /></div>
    </div>
    {#if mcp}
      <div class="setting">
        <div class="what"><b>{t.mcp}</b><span>{t.mcpHint}</span></div>
        <div class="how">
          <span class="state"><span class="dot {mcp.registered ? 'done' : ''}"></span>{mcp.registered ? t.connected : t.notConnected}</span>
          <button class="btn small" onclick={() => registerMcp(!mcp!.registered)}>{mcp.registered ? t.disconnect : t.connect}</button>
        </div>
      </div>
    {/if}
  </section>
  {#if mcp?.command}
    <details class="more">
      <summary class="section-title">{t.advanced}</summary>
      <section class="panel">
        <div class="setting">
          <div class="what"><b>{t.mcpCommand}</b><span class="mono">{mcp.command} {mcp.args.join(" ")}</span></div>
        </div>
      </section>
    </details>
  {/if}
{/snippet}

{#snippet aboutPage()}
  <section class="panel">
    <div class="setting">
      <div class="what"><b>{t.data}</b><span class="mono">{store.info?.dir ?? "—"}</span></div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.setupAgain}</b></div>
      <div class="how"><button class="btn small" onclick={setupAgain}>{t.setupRun}</button></div>
    </div>
  </section>
{/snippet}

<style>
  /* The wiring. */
  .stage {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 12px;
    margin-top: 8px;
  }
  /* Wide enough: Yoru in the middle, a column of cards either side, and
     room between for the traces. */
  .stage.wired {
    grid-template-columns: minmax(200px, 1fr) auto minmax(200px, 1fr);
    column-gap: clamp(44px, 8%, 96px);
    align-items: center;
    min-height: 340px;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  /* Narrow: every card in one grid under Yoru, rows lined up. */
  .stage:not(.wired) .col {
    display: contents;
  }
  .stage.wired .col.left {
    grid-column: 1;
    grid-row: 1;
  }
  .stage.wired .col:not(.left) {
    grid-column: 3;
    grid-row: 1;
  }
  .wires {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
    pointer-events: none;
  }
  .copper {
    fill: none;
    stroke: color-mix(in srgb, var(--c) 30%, var(--ink-3));
    stroke-width: 2;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
    stroke-dasharray: 1;
    stroke-dashoffset: 1;
    animation: draw 700ms cubic-bezier(0.3, 0.7, 0.2, 1) forwards;
    animation-delay: calc(var(--i) * 90ms + 120ms);
    transition:
      stroke 220ms,
      filter 220ms;
  }
  /* A spark runs along each trace into Yoru; faster on the one you point at. */
  .pulse {
    fill: none;
    stroke: var(--c);
    stroke-width: 2.5;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
    stroke-dasharray: 5 95;
    opacity: 0.45;
    animation:
      flow 3.2s linear calc(var(--i) * -530ms) infinite,
      appear 500ms ease 1s backwards;
    filter: drop-shadow(0 0 4px var(--c));
    transition: opacity 300ms;
  }
  .pad {
    fill: var(--night);
    stroke: color-mix(in srgb, var(--c) 45%, var(--ink-3));
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
    transition: stroke 220ms;
  }
  .wire.lit .copper {
    stroke: var(--c);
    filter: drop-shadow(0 0 6px var(--c));
  }
  .wire.lit .pulse {
    opacity: 1;
    stroke-width: 3;
    animation-duration: 1.4s, 500ms;
  }
  .wire.lit .pad {
    stroke: var(--c);
  }
  .wire.warn {
    --c: var(--wrong);
  }

  .yoru {
    position: relative;
    display: grid;
    place-items: center;
    justify-self: center;
  }
  .stage:not(.wired) .yoru {
    grid-column: 1 / -1;
    grid-row: 1;
  }
  .stage.wired .yoru {
    grid-column: 2;
    grid-row: 1;
  }

  /* A part: a card wired to Yoru. */
  .more-title {
    margin-top: 28px;
  }
  .menu {
    display: grid;
    padding: 4px;
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 0;
    border-radius: calc(var(--r-lg) - 4px);
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 160ms;
  }
  .menu button + button {
    box-shadow: 0 -1px 0 var(--rim);
  }
  .menu button:hover,
  .menu button:focus-visible {
    background: color-mix(in srgb, var(--c) 7%, var(--ink-2));
    outline: none;
  }
  .part {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    animation: arrive 480ms cubic-bezier(0.2, 0.8, 0.2, 1) calc(var(--i) * 90ms) backwards;
    transition:
      border-color 200ms,
      background 200ms,
      box-shadow 200ms;
  }
  .part:hover,
  .part:focus-visible {
    border-color: color-mix(in srgb, var(--c) 55%, transparent);
    background: color-mix(in srgb, var(--c) 7%, var(--ink));
    box-shadow: 0 0 28px -8px color-mix(in srgb, var(--c) 60%, transparent);
    outline: none;
  }
  .part.warn {
    --c: var(--wrong);
    border-color: color-mix(in srgb, var(--wrong) 35%, transparent);
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    border-radius: 50%;
    background: color-mix(in srgb, var(--c) 14%, transparent);
    color: var(--c);
  }
  .glyph.big {
    width: 56px;
    height: 56px;
    box-shadow: 0 0 40px -10px var(--c);
  }
  .words {
    display: grid;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .words {
    color: var(--pearl);
    font-size: var(--fs-body);
  }
  .words b {
    display: flex;
    align-items: center;
    gap: 7px;
    font: 600 var(--fs-body)/1.3 var(--sans);
  }
  .says {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    color: var(--haze);
    font-size: var(--fs-meta);
    line-height: 1.35;
  }
  .chev {
    flex-shrink: 0;
    color: var(--dusk);
    transition:
      stroke 200ms,
      transform 200ms;
  }
  .menu button:hover .chev,
  .part:hover .chev {
    color: var(--c);
    transform: translateX(2px);
  }

  /* A part's page. */
  .back {
    margin-left: -10px;
    gap: 4px;
  }
  .sub-head {
    display: flex;
    align-items: center;
    gap: 18px;
    margin: 14px 0 18px;
  }
  .sub-head h1 {
    font: 600 var(--fs-title)/1.1 var(--display);
    letter-spacing: -0.01em;
  }
  .sub-head .lede {
    margin-top: 6px;
    color: var(--haze);
    max-width: 580px;
  }
  .hop {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 22px;
  }

  .perm-hint {
    margin: 10px 0 18px;
  }
  .small {
    font-size: var(--fs-meta);
    margin: -4px 0 10px;
  }
  .panel .small {
    margin: 4px 0 8px;
  }
  .langs {
    grid-template-columns: repeat(auto-fill, minmax(160px, 220px));
  }
  .note {
    margin-top: 12px;
  }

  .block {
    display: grid;
    gap: 12px;
    padding: 6px 0 16px;
  }
  .block + .setting,
  .block + .block {
    border-top: 1px solid var(--rim);
  }
  .block + .block {
    padding-top: 16px;
  }
  .block .what {
    display: grid;
    gap: 2px;
  }
  .block .what b {
    font-weight: 550;
  }
  .block .what span {
    font-size: var(--fs-meta);
  }
  .model {
    width: 220px;
  }
  select {
    max-width: 340px;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--haze);
    font-size: var(--fs-meta);
    margin-right: 4px;
    white-space: nowrap;
  }

  @keyframes draw {
    to {
      stroke-dashoffset: 0;
    }
  }
  @keyframes flow {
    from {
      stroke-dashoffset: 100;
    }
    to {
      stroke-dashoffset: 0;
    }
  }
  @keyframes appear {
    from {
      opacity: 0;
    }
  }
  @keyframes arrive {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
</style>
