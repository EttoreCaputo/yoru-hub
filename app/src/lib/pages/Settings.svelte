<!--
  Settings as Yoru's wiring: the mirror in the middle, and each part of the
  settings a card wired to it by a trace, with what it says right now. A card
  opens its own page: Yoru's brain (Groq, OpenAI, Claude or Ollama, their keys
  and models, how hard Claude thinks), Claude Code's integration, Yoru's voice
  with the cloud voices' keys, the app's language, the board (connection,
  firmware, WiFi, its own settings) and the data folder.
-->
<script lang="ts">
  import { fly } from "svelte/transition";
  import { call, why } from "../api";
  import { i18n, LANGUAGES, type Lang } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Brain, ClaudeAlertsInfo, ClaudeLink, Effort, Gaze, McpInfo, VoiceInfo } from "../types";
  import YoruBoard from "../YoruBoard.svelte";
  import Board from "../ui/Board.svelte";
  import KeyField from "../ui/KeyField.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import Voice from "../ui/Voice.svelte";

  const t = $derived(i18n.t.settings);
  const s = $derived(store.info?.settings);
  const EFFORTS: Effort[] = ["low", "medium", "high", "xhigh", "max"];
  const CLAUDE_MODELS = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
  const brain = $derived(store.info?.brain ?? null);
  const BRAIN_NAMES: Record<Brain, string> = { anthropic: "Claude", openai: "OpenAI", groq: "Groq", ollama: "Ollama" };
  /** The brain picker's tiles; null lets the app choose. */
  const BRAIN_CHOICES: (Brain | null)[] = [null, "anthropic", "openai", "groq", "ollama"];
  const LANG_CHOICES: (Lang | null)[] = [null, ...LANGUAGES.map((l) => l.id)];
  const langName = (id: Lang) => LANGUAGES.find((l) => l.id === id)?.name ?? id;

  let voice = $state<VoiceInfo | null>(null);
  let alerts = $state<ClaudeAlertsInfo | null>(null);
  let link = $state<ClaudeLink | null>(null);
  let mcp = $state<McpInfo | null>(null);
  let error = $state("");

  async function readAll() {
    [voice, alerts, link, mcp] = await Promise.all([
      call<VoiceInfo>("voice_info").catch(() => null),
      call<ClaudeAlertsInfo>("claude_alerts_info").catch(() => null),
      call<ClaudeLink>("claude_link").catch(() => null),
      call<McpInfo>("mcp_info").catch(() => null),
    ]);
  }
  $effect(() => {
    void readAll();
  });

  // The brains' keys (Groq's and OpenAI's serve speech to text too) are in the brain's page.
  const BRAINS = ["anthropic", "openai", "groq", "ollama"];
  const others = $derived(voice?.providers.filter((p) => !BRAINS.includes(p.id)) ?? []);

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
    if (s?.provider === "ollama") void readOllama();
  });

  async function run<T>(job: () => Promise<T>, then: (v: T) => void) {
    error = "";
    try {
      then(await job());
    } catch (e) {
      error = why(e);
    }
  }
  const hooks = (connect: boolean) => run(() => call<ClaudeAlertsInfo>("claude_hooks", { connect }), (v) => (alerts = v));
  const statusLine = (connect: boolean) => run(() => call<ClaudeLink>(connect ? "claude_connect" : "claude_disconnect"), (v) => (link = v));
  const registerMcp = (connect: boolean) => run(() => call<McpInfo>("claude_mcp", { connect }), (v) => (mcp = v));
  const setAlerts = (change: Partial<ClaudeAlertsInfo["settings"]>) =>
    alerts && run(() => call<ClaudeAlertsInfo>("set_claude_alerts", { settings: { ...$state.snapshot(alerts!.settings), ...change } }), (v) => (alerts = v));

  async function setupAgain() {
    await call("set_onboarding", { done: false, tourVersion: null });
    store.onboarding = { done: false, tour_version: store.onboarding?.tour_version ?? 0 };
  }

  // The parts, wired to Yoru: three on each side, top to bottom.
  type Part = "brain" | "claude" | "voice" | "language" | "board" | "about";
  type Wire = { id: Part; side: -1 | 1; row: 0 | 1 | 2; color: string; icon: string };
  const WIRES: Wire[] = [
    { id: "brain", side: -1, row: 0, color: "var(--think)", icon: "M9 4.5a3 3 0 0 0-3 3 3 3 0 0 0-1.5 5.5A3 3 0 0 0 9 18.5M15 4.5a3 3 0 0 1 3 3 3 3 0 0 1 1.5 5.5 3 3 0 0 1-4.5 5.5M12 4v16M9 10h3M12 14h3" },
    { id: "voice", side: -1, row: 1, color: "#c58cff", icon: "M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2" },
    { id: "language", side: -1, row: 2, color: "#5fd4c8", icon: "M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17ZM3.5 12h17M12 3.5c2.4 2.4 3.5 5.2 3.5 8.5s-1.1 6.1-3.5 8.5M12 3.5C9.6 5.9 8.5 8.7 8.5 12s1.1 6.1 3.5 8.5" },
    { id: "claude", side: 1, row: 0, color: "var(--wait)", icon: "M8 8l-4 4 4 4M16 8l4 4-4 4M13.5 5.5l-3 13" },
    { id: "board", side: 1, row: 1, color: "var(--done)", icon: "M7 4h10a3 3 0 0 1 3 3v10a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3ZM9.5 9.5h5v5h-5zM9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2" },
    { id: "about", side: 1, row: 2, color: "var(--pearl-mid)", icon: "M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17ZM12 11v5.5M12 7.6v.1" },
  ];
  const PAGES: Part[] = ["brain", "claude", "voice", "language", "board", "about"];

  let open = $state<Part | null>(null);
  let hover = $state<Part | null>(null);
  const wire = (id: Part) => WIRES.find((w) => w.id === id)!;

  function pick(id: Part, e: Event) {
    hover = null;
    open = id;
    (e.currentTarget as HTMLElement).closest("main")?.scrollTo({ top: 0 });
  }

  // What each card says now, and its dot.
  const brainModel = $derived(!s || !brain ? "" : brain === "anthropic" ? s.model : brain === "openai" ? s.openai_model : brain === "groq" ? s.groq_model : s.ollama_model);
  const links = $derived([alerts?.hooks.connected, link?.connected, mcp?.registered].filter((v) => v !== undefined));
  const linked = $derived(links.filter(Boolean).length);
  const keyed = $derived(others.filter((p) => p.key).length);
  const firmware = $derived(store.firmware?.state === "version" ? store.firmware.version : null);
  const via = $derived(store.port?.startsWith("wifi:") ? i18n.t.link.wifi : store.port ? i18n.t.link.usb : "");
  const summary = $derived<Record<Part, { text: string; dot: string; warn?: boolean }>>({
    brain: !store.info?.ready
      ? { text: brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain, dot: "wrong", warn: true }
      : { text: `${brain ? BRAIN_NAMES[brain] : ""}${brainModel ? " · " + brainModel : ""}`, dot: store.info?.limited ? "wait" : "done" },
    claude: { text: links.length ? t.linksOf(linked, links.length) : "—", dot: !links.length || !linked ? "" : linked === links.length ? "done" : "wait" },
    voice: {
      text: [voice ? (voice.settings.enabled ? t.listening : t.notListening) : "", others.length ? (keyed ? t.keysOf(keyed, others.length) : t.noKeys) : ""].filter(Boolean).join(" · ") || "—",
      dot: voice?.settings.enabled ? "done" : "",
    },
    language: { text: i18n.choice ? langName(i18n.choice) : `${t.system} · ${langName(i18n.lang)}`, dot: "" },
    board:
      store.boardState === "off"
        ? { text: t.boardOff, dot: "" }
        : { text: [store.boardState === "boot" ? i18n.t.link.boot : i18n.t.link.live, via, firmware].filter(Boolean).join(" · "), dot: store.update ? "wait" : store.boardState === "boot" ? "think" : "done" },
    about: { text: t.aboutSummary, dot: "" },
  });

  // The wiring is drawn on a 1000 × 500 stage; the cards take the outer 31%.
  let stageW = $state(840);
  const yoruSize = $derived(Math.max(110, Math.min(180, stageW * 0.2)));
  /** The case's radius, in stage units. */
  const rim = $derived(((yoruSize * 1.15) / 2) * (1000 / stageW) * 0.96);
  const ROW_Y = [75, 250, 425];
  function trace(w: Wire) {
    const ax = 500 + w.side * 190;
    const ay = ROW_Y[w.row];
    const a = w.row === 1 ? 0 : Math.PI / 4;
    const ex = 500 + w.side * rim * Math.cos(a);
    const ey = 250 + (w.row - 1) * rim * Math.sin(a);
    // Out of the card flat, then in at 45°, as copper runs on a board.
    const bx = w.side < 0 ? Math.max(ax, ex - Math.abs(ey - ay)) : Math.min(ax, ex + Math.abs(ey - ay));
    return { d: `M ${ax} ${ay} H ${bx} L ${ex} ${ey}`, ax, ay, ex, ey };
  }
  // Yoru looks at the card under the pointer.
  const gaze = $derived<Gaze | null>(hover ? { x: wire(hover).side * 0.9, y: (wire(hover).row - 1) * 0.75, movedAt: store.now } : store.gaze);
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
      {:else if open === "claude"}{@render claudePage()}
      {:else if open === "voice"}{@render voicePage()}
      {:else if open === "language"}{@render languagePage()}
      {:else if open === "board"}<Board />
      {:else}{@render aboutPage()}{/if}
    </div>
  {:else}
    <div class="page-head">
      <div>
        <h1>{t.title}</h1>
        <p class="lede">{t.lede}</p>
      </div>
    </div>

    <div class="stage" bind:clientWidth={stageW} style:height="{stageW / 2}px">
      <svg class="wires" viewBox="0 0 1000 500" aria-hidden="true">
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
          style:top="{ROW_Y[w.row] / 5}%"
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

{#snippet brainPage()}
  {#if s}
    <section class="tiles" aria-label={t.provider}>
      {#each BRAIN_CHOICES as b (b ?? "auto")}
        <button class="tile" class:on={(s.provider ?? null) === b} onclick={() => store.saveSettings((x) => (x.provider = b))}>
          <b>{b ? BRAIN_NAMES[b] : t.autoBrain}</b>
          <span>{t.brainNotes[b ?? "auto"]}</span>
          {#if b && brain === b && store.info?.ready}<span class="tag">{t.inUse}</span>{/if}
        </button>
      {/each}
    </section>
    {#if !store.info?.ready}
      <p class="error-text note">{brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain}</p>
    {:else if store.info?.limited}
      <p class="error-text note">{store.info.limited}</p>
    {/if}
  {/if}

  <h2 class="section-title">Claude</h2>
  <section class="panel" class:active={brain === "anthropic"}>
    <div class="block">
      <div class="what"><b>{t.anthropic}</b><span class="dim">{t.anthropicHint}</span></div>
      <KeyField provider="anthropic" source={store.info?.key ?? null} placeholder={t.keyPlaceholder} onchange={() => store.refreshInfo()} />
    </div>
    {#if s}
      <div class="setting">
        <div class="what"><b>{t.claudeModel}</b></div>
        <div class="how">
          <input class="model mono" type="text" list="claude-models" value={s.model} spellcheck="false" onchange={(e) => store.saveSettings((x) => (x.model = e.currentTarget.value.trim() || x.model))} aria-label={t.claudeModel} />
          <datalist id="claude-models">{#each CLAUDE_MODELS as m (m)}<option value={m}></option>{/each}</datalist>
        </div>
      </div>
    {/if}
  </section>

  <h2 class="section-title">OpenAI</h2>
  <section class="panel" class:active={brain === "openai"}>
    <div class="block">
      <div class="what"><b>{t.openai}</b><span class="dim">{t.openaiHint}</span></div>
      <KeyField provider="openai" source={store.info?.openai_key ?? null} placeholder="sk-proj-…" onchange={() => (store.refreshInfo(), readAll())} />
    </div>
    {#if s}
      <div class="setting">
        <div class="what"><b>{t.openaiModel}</b>{#if brain === "openai"}<span>{t.groqNote}</span>{/if}</div>
        <div class="how">
          <select value={s.openai_model} onchange={(e) => store.saveSettings((x) => (x.openai_model = e.currentTarget.value))} aria-label={t.openaiModel}>
            {#each store.info?.openai_models ?? [] as m (m.id)}<option value={m.id}>{m.id} — {m.note}</option>{/each}
          </select>
        </div>
      </div>
    {/if}
  </section>

  <h2 class="section-title">Groq</h2>
  <section class="panel" class:active={brain === "groq"}>
    <div class="block">
      <div class="what"><b>{t.groq}</b><span class="dim">{t.groqHint}</span></div>
      <KeyField provider="groq" source={store.info?.groq_key ?? null} placeholder="gsk_…" onchange={() => (store.refreshInfo(), readAll())} />
    </div>
    {#if s}
      <div class="setting">
        <div class="what"><b>{t.groqModel}</b>{#if brain === "groq"}<span>{t.groqNote}</span>{/if}</div>
        <div class="how">
          <select value={s.groq_model} onchange={(e) => store.saveSettings((x) => (x.groq_model = e.currentTarget.value))} aria-label={t.groqModel}>
            {#each store.info?.groq_models ?? [] as m (m.id)}<option value={m.id}>{m.id} — {m.note}</option>{/each}
          </select>
        </div>
      </div>
    {/if}
  </section>

  {#if s}
    <h2 class="section-title">{t.ollama}</h2>
    <section class="panel" class:active={brain === "ollama"}>
      <div class="setting">
        <div class="what">
          <b>{t.ollamaModel}</b><span class="dim">{t.ollamaHint}</span>
          {#if brain === "ollama"}<span>{t.groqNote}</span>{/if}
          {#if ollamaError}<span class="error-text">{ollamaError}</span>{:else if ollamaModels && !ollamaModels.length}<span class="error-text">{t.ollamaNone}</span>{/if}
        </div>
        <div class="how">
          <input class="model mono" type="text" list="ollama-models" value={s.ollama_model} placeholder={t.ollamaPick} spellcheck="false" onchange={(e) => store.saveSettings((x) => (x.ollama_model = e.currentTarget.value.trim()))} aria-label={t.ollamaModel} />
          <datalist id="ollama-models">{#each ollamaModels ?? [] as m (m)}<option value={m}></option>{/each}</datalist>
          <button class="btn small" onclick={readOllama}>{t.ollamaRefresh}</button>
          <button class="btn small" onclick={() => call("open_key_page", { provider: "ollama" })}>{t.ollamaGet}</button>
        </div>
      </div>
    </section>

    <h2 class="section-title">{t.howItThinks}</h2>
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.effort}</b>{#if brain !== "anthropic"}<span>{t.claudeOnly}</span>{/if}</div>
        <div class="how">
          <select value={s.effort} onchange={(e) => store.saveSettings((x) => (x.effort = e.currentTarget.value as Effort))} aria-label={t.effort}>
            {#each EFFORTS as e (e)}<option value={e}>{t.efforts[e]}</option>{/each}
          </select>
        </div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.taskEffort}</b>{#if brain !== "anthropic"}<span>{t.claudeOnly}</span>{/if}</div>
        <div class="how">
          <select value={s.task_effort} onchange={(e) => store.saveSettings((x) => (x.task_effort = e.currentTarget.value as Effort))} aria-label={t.taskEffort}>
            {#each EFFORTS as e (e)}<option value={e}>{t.efforts[e]}</option>{/each}
          </select>
        </div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.webSearch}</b>{#if brain !== "anthropic"}<span>{t.claudeOnly}</span>{/if}</div>
        <div class="how"><Toggle checked={s.web_search} label={t.webSearch} onchange={(on) => store.saveSettings((x) => (x.web_search = on))} /></div>
      </div>
    </section>
  {/if}
{/snippet}

{#snippet claudePage()}
  <section class="panel">
    {#if alerts}
      <div class="setting">
        <div class="what"><b>{t.hooks}</b><span>{t.hooksHint}</span></div>
        <div class="how">
          <span class="state"><span class="dot {alerts.hooks.connected ? 'done' : ''}"></span>{alerts.hooks.connected ? t.connected : t.notConnected}</span>
          <button class="btn small" onclick={() => hooks(!alerts!.hooks.connected)}>{alerts.hooks.connected ? t.disconnect : t.connect}</button>
        </div>
      </div>
    {/if}
    {#if link}
      <div class="setting">
        <div class="what"><b>{t.statusLine}</b><span>{t.statusLineHint}</span></div>
        <div class="how">
          <span class="state"><span class="dot {link.connected ? 'done' : ''}"></span>{link.connected ? t.connected : t.notConnected}</span>
          <button class="btn small" onclick={() => statusLine(!link!.connected)}>{link.connected ? t.disconnect : t.connect}</button>
        </div>
      </div>
    {/if}
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
  {#if alerts}
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.approvals}</b><span>{t.approvalsHint}</span></div>
        <div class="how"><Toggle checked={alerts.settings.approvals} label={t.approvals} onchange={(on) => setAlerts({ approvals: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.alertsSpeak}</b></div>
        <div class="how"><Toggle checked={alerts.settings.speak} label={t.alertsSpeak} onchange={(on) => setAlerts({ speak: on })} /></div>
      </div>
    </section>
  {/if}
  {#if error}<p class="error-text note">{error}</p>{/if}
{/snippet}

{#snippet voicePage()}
  <Voice />
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
  .block + .setting {
    border-top: 1px solid var(--rim);
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
