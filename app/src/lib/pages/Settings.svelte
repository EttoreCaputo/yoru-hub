<!--
  Settings as Yoru's wiring: the mirror in the middle, and each part of the
  settings a card wired to him by a trace, with what it says right now. On
  the left what Yoru is (his brain, his character, his voice, the language
  he answers in); on the right what he's connected to (the services' keys,
  the Mac, the board, the data folder). A card opens its own page, with the
  everyday choices first and the technical ones under "Advanced".
-->
<script lang="ts">
  import { fly } from "svelte/transition";
  import { call, why } from "../api";
  import { i18n, LANGUAGES, type Lang } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { AssistantInfo, AssistantSettings, Brain, Effort, Gaze, KeySource, McpInfo, VoiceInfo, VoiceSettings } from "../types";
  import YoruBoard from "../YoruBoard.svelte";
  import Board from "../ui/Board.svelte";
  import KeyField from "../ui/KeyField.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import Voice from "../ui/Voice.svelte";
  import Profiles from "../ui/Profiles.svelte";
  import { currentProfile } from "../profiles";

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
  let error = $state("");

  async function readAll() {
    [voice, mcp] = await Promise.all([call<VoiceInfo>("voice_info").catch(() => null), call<McpInfo>("mcp_info").catch(() => null)]);
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

  // The levels of core/src/character.rs, friendly to assistant; any other id is the default.
  const CHARACTERS = ["friend", "confidant", "analyst", "advisor", "assistant"];
  const character = $derived(CHARACTERS.includes(store.info?.settings.character ?? "") ? store.info!.settings.character : "confidant");
  const setCharacter = (c: string) => store.saveSettings((x: AssistantSettings) => (x.character = c));

  async function setupAgain() {
    await call("set_onboarding", { done: false, tourVersion: null });
    store.onboarding = { done: false, tour_version: store.onboarding?.tour_version ?? 0 };
  }

  // The parts, wired to Yoru: four on each side, top to bottom.
  type Part = "brain" | "character" | "voice" | "language" | "keys" | "mac" | "board" | "about";
  type Wire = { id: Part; side: -1 | 1; row: 0 | 1 | 2 | 3; color: string; icon: string };
  const WIRES: Wire[] = [
    { id: "brain", side: -1, row: 0, color: "var(--think)", icon: "M9 4.5a3 3 0 0 0-3 3 3 3 0 0 0-1.5 5.5A3 3 0 0 0 9 18.5M15 4.5a3 3 0 0 1 3 3 3 3 0 0 1 1.5 5.5 3 3 0 0 1-4.5 5.5M12 4v16M9 10h3M12 14h3" },
    { id: "character", side: -1, row: 1, color: "#ff8fb8", icon: "M12 20.5s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7.6a4.3 4.3 0 0 1 7.5 2.7c0 5.6-7.5 10.2-7.5 10.2Z" },
    { id: "voice", side: -1, row: 2, color: "#c58cff", icon: "M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2" },
    { id: "language", side: -1, row: 3, color: "#5fd4c8", icon: "M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17ZM3.5 12h17M12 3.5c2.4 2.4 3.5 5.2 3.5 8.5s-1.1 6.1-3.5 8.5M12 3.5C9.6 5.9 8.5 8.7 8.5 12s1.1 6.1 3.5 8.5" },
    { id: "keys", side: 1, row: 0, color: "var(--wait)", icon: "M14.5 9.5a4 4 0 1 0-3.6 3.98L5 19.5V21h3v-2h2v-2h1.5l1.7-1.7M15.5 7.5h.01" },
    { id: "mac", side: 1, row: 1, color: "#7fb2ff", icon: "M4 5.5h16a1 1 0 0 1 1 1V16a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6.5a1 1 0 0 1 1-1ZM9.5 5.5v1.2a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1V5.5M8 20.5h8M12 17v3.5" },
    { id: "board", side: 1, row: 2, color: "var(--done)", icon: "M7 4h10a3 3 0 0 1 3 3v10a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3ZM9.5 9.5h5v5h-5zM9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2" },
    { id: "about", side: 1, row: 3, color: "var(--pearl-mid)", icon: "M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17ZM12 11v5.5M12 7.6v.1" },
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
  const summary = $derived<Record<Part, { text: string; dot: string; warn?: boolean }>>({
    brain: !store.info?.ready
      ? { text: brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain, dot: "wrong", warn: true }
      : { text: profile ? i18n.t.profiles.names[profile] : `${t.customShort} · ${brain ? BRAIN_NAMES[brain] : ""}${brainModel ? " · " + brainModel : ""}`, dot: store.info?.limited ? "wait" : "done" },
    character: { text: store.info ? t.characters[character][0] : "—", dot: "" },
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
    about: { text: t.aboutSummary, dot: "" },
  });

  // The wiring is drawn on a 1000 × 560 stage; the cards take the outer 31%.
  const H = 560;
  const ROW_Y = [70, 210, 350, 490];
  /** Where each row's trace meets the case, from straight up (−90°) to straight down. */
  const ROW_ANGLE = [-48, -16, 16, 48].map((d) => (d * Math.PI) / 180);
  let stageW = $state(840);
  const yoruSize = $derived(Math.max(110, Math.min(180, stageW * 0.2)));
  /** The case's radius, in stage units. */
  const rim = $derived(((yoruSize * 1.15) / 2) * (1000 / stageW) * 0.96);
  function trace(w: Wire) {
    const ax = 500 + w.side * 190;
    const ay = ROW_Y[w.row];
    const a = ROW_ANGLE[w.row];
    const ex = 500 + w.side * rim * Math.cos(a);
    const ey = H / 2 + rim * Math.sin(a);
    // Out of the card flat, then in at 45°, as copper runs on a board.
    const bx = w.side < 0 ? Math.max(ax, ex - Math.abs(ey - ay)) : Math.min(ax, ex + Math.abs(ey - ay));
    return { d: `M ${ax} ${ay} H ${bx} L ${ex} ${ey}`, ax, ay, ex, ey };
  }
  // Yoru looks at the card under the pointer.
  const gaze = $derived<Gaze | null>(hover ? { x: wire(hover).side * 0.9, y: (wire(hover).row - 1.5) * 0.5, movedAt: store.now } : store.gaze);
</script>

<div class="page">
  {#if open}
    {@const w = wire(open)}
    <div class="sub" style:--c={w.color} in:fly={{ y: 10, duration: 260 }}>
      <button class="btn quiet small back" onclick={() => ((open = null), readAll())}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14.5 6l-6 6 6 6" /></svg>{t.back}
      </button>
      <div class="sub-head">
        <span class="glyph big"><svg viewBox="0 0 24 24" aria-hidden="true"><path d={w.icon} /></svg></span>
        <div>
          <h1>{t.parts[open]}</h1>
          <p class="lede">{t.partLedes[open]}</p>
        </div>
      </div>
      <nav class="hop" aria-label={t.title}>
        {#each PAGES as p (p)}
          <button class:on={open === p} style:--c={wire(p).color} onclick={(e) => pick(p, e)}>{t.parts[p]}</button>
        {/each}
      </nav>
      {#if open === "brain"}{@render brainPage()}
      {:else if open === "character"}{@render characterPage()}
      {:else if open === "voice"}<Voice />
      {:else if open === "language"}{@render languagePage()}
      {:else if open === "keys"}{@render keysPage()}
      {:else if open === "mac"}{@render macPage()}
      {:else if open === "board"}<Board />
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

    <div class="stage" bind:clientWidth={stageW} style:height="{(stageW * H) / 1000}px">
      <svg class="wires" viewBox="0 0 1000 {H}" aria-hidden="true">
        {#each WIRES as w, i (w.id)}
          {@const p = trace(w)}
          <g class="wire" class:lit={hover === w.id} class:warn={summary[w.id].warn} style:--c={w.color} style:--i={i}>
            <path class="copper" d={p.d} pathLength="1" />
            <path class="pulse" d={p.d} pathLength="100" />
            <circle class="pad" cx={p.ax} cy={p.ay} r="5" />
            <circle class="pad" cx={p.ex} cy={p.ey} r="4" />
          </g>
        {/each}
      </svg>

      <div class="yoru">
        <YoruBoard board={store.boardState === "off" ? "off" : "live"} size={yoruSize} voice={store.voice} {gaze} />
      </div>

      {#each WIRES as w, i (w.id)}
        {@const sum = summary[w.id]}
        <button
          class="part"
          class:left={w.side < 0}
          class:warn={sum.warn}
          style:--c={w.color}
          style:--i={i}
          style:top="{(ROW_Y[w.row] / H) * 100}%"
          onmouseenter={() => (hover = w.id)}
          onmouseleave={() => (hover = null)}
          onfocus={() => (hover = w.id)}
          onblur={() => (hover = null)}
          onclick={(e) => pick(w.id, e)}
        >
          <span class="glyph"><svg viewBox="0 0 24 24" aria-hidden="true"><path d={w.icon} /></svg></span>
          <span class="words">
            <b>{t.parts[w.id]}{#if sum.dot}<span class="dot {sum.dot}"></span>{/if}</b>
            <span class="says" class:error-text={sum.warn} title={sum.text}>{sum.text}</span>
          </span>
          <svg class="chev" viewBox="0 0 24 24" aria-hidden="true"><path d="M9.5 6l6 6-6 6" /></svg>
        </button>
      {/each}
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

<!-- Who Yoru is: a level on one scale, friendly to assistant, for every brain. -->
{#snippet characterPage()}
  {#if store.info}
    <div class="scale faint" aria-hidden="true">
      <span>{t.scale[0]}</span><span>{t.scale[1]}</span><span>{t.scale[2]}</span>
    </div>
    <section class="tiles levels" aria-label={t.parts.character}>
      {#each CHARACTERS as c, i (c)}
        <button class="tile" class:on={character === c} onclick={() => setCharacter(c)}>
          <b>{i + 1} · {t.characters[c][0]}</b>
          <span>{t.characters[c][1]}</span>
        </button>
      {/each}
    </section>
    <p class="dim note">{t.characterHint}</p>
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
    margin-top: 8px;
  }
  .wires {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
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
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
  }

  /* A part: a card wired to Yoru. */
  .part {
    position: absolute;
    right: 0;
    width: 31%;
    min-width: 170px;
    transform: translateY(-50%);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border-radius: var(--r-lg);
    border: 1px solid var(--rim);
    background: var(--ink);
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
    animation: arrive 480ms cubic-bezier(0.2, 0.8, 0.2, 1) calc(var(--i) * 90ms) backwards;
    transition:
      border-color 200ms,
      background 200ms,
      box-shadow 200ms;
  }
  .part.left {
    right: auto;
    left: 0;
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
  .glyph svg {
    width: 20px;
    height: 20px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .glyph.big {
    width: 56px;
    height: 56px;
    box-shadow: 0 0 40px -10px var(--c);
  }
  .glyph.big svg {
    width: 28px;
    height: 28px;
  }
  .words {
    display: grid;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .words b {
    display: flex;
    align-items: center;
    gap: 7px;
    font-weight: 600;
  }
  .says {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    color: var(--haze);
    font-size: 12px;
    line-height: 1.35;
  }
  .chev {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    fill: none;
    stroke: var(--dusk);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition:
      stroke 200ms,
      transform 200ms;
  }
  .part:hover .chev {
    stroke: var(--c);
    transform: translateX(2px);
  }

  /* A part's page. */
  .back {
    margin-left: -10px;
    gap: 4px;
  }
  .back svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .sub-head {
    display: flex;
    align-items: center;
    gap: 18px;
    margin: 14px 0 18px;
  }
  .sub-head h1 {
    font: 600 30px/1.1 var(--display);
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
  .hop button {
    height: 28px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--rim);
    background: transparent;
    color: var(--haze);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
    transition:
      color 160ms,
      border-color 160ms,
      background 160ms;
  }
  .hop button:hover {
    color: var(--pearl);
  }
  .hop button.on {
    color: var(--c);
    border-color: color-mix(in srgb, var(--c) 45%, transparent);
    background: color-mix(in srgb, var(--c) 10%, transparent);
  }
  .panel.active {
    border-color: color-mix(in srgb, var(--c) 40%, transparent);
    box-shadow: 0 0 36px -16px var(--c);
  }

  /* The five levels side by side, under the scale they sit on. */
  .levels {
    grid-template-columns: repeat(5, minmax(0, 1fr));
  }
  .scale {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
    margin-bottom: 8px;
    padding: 0 4px;
  }
  .small {
    font-size: 12.5px;
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
    font-size: 12.5px;
  }
  .model {
    width: 220px;
  }
  select {
    width: auto;
    min-width: 180px;
    max-width: 340px;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--haze);
    font-size: 12px;
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
      transform: translateY(calc(-50% + 8px));
    }
  }
</style>
