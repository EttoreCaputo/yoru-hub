<!--
  Settings, sorted by what you want done rather than by how Yoru is built.
  On top what's wrong right now, each with the button that leads to the fix,
  or that all is well; then four quick settings (his voice, the language,
  the brain, do not disturb); then the rest as rows in four groups: Yoru,
  Devices, Accounts and services, General. Every setting lives in one place
  only. A row opens its page with the everyday choices; models, ports and
  MCP show only with "Show advanced options", one switch for every page.
-->
<script lang="ts">
  import { look, THEMES } from "../look.svelte";
  import { fly } from "svelte/transition";
  import { call, why } from "../api";
  import { i18n, LANGUAGES, type Lang } from "../i18n/index.svelte";
  import { store, type Page } from "../store.svelte";
  import type { AssistantInfo, AssistantSettings, Brain, Effort, KeySource, McpInfo, MeetingSettings, VoiceInfo } from "../types";
  import YoruBoard from "../YoruBoard.svelte";
  import Board from "../ui/Board.svelte";
  import KeyField from "../ui/KeyField.svelte";
  import PermissionList from "../ui/PermissionList.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import Voice from "../ui/Voice.svelte";
  import MeetingSettingsPage from "../ui/MeetingSettings.svelte";
  import PocketSettingsPage from "../ui/PocketSettings.svelte";
  import Profiles from "../ui/Profiles.svelte";
  import { transcriptionRequirement } from "../meeting-readiness";
  import { PAGE_ICONS } from "../icons/pages";
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

  async function setupAgain() {
    await call("set_onboarding", { done: false, tourVersion: null });
    store.onboarding = { done: false, tour_version: store.onboarding?.tour_version ?? 0 };
  }

  // The parts with a page of their own here; the other rows open an app page.
  type Part = "brain" | "voice" | "meetings" | "keys" | "mac" | "board" | "pocket";
  const PART_ICONS: Record<Part, IconName> = { brain: "message-circle", voice: "audio-waveform", meetings: "users", keys: "key-round", mac: "laptop", board: "cpu", pocket: "mic" };
  type Row = { part: Part } | { page: Page };
  const GROUPS: { id: string; rows: Row[] }[] = [
    { id: "yoru", rows: [{ part: "brain" }, { part: "voice" }, { part: "meetings" }, { page: "routines" }, { page: "tasks" }] },
    { id: "devices", rows: [{ part: "board" }, { part: "pocket" }, { part: "mac" }] },
    { id: "accounts", rows: [{ page: "connectors" }, { part: "keys" }] },
  ];

  let open = $state<Part | null>(store.settingsSection);
  store.settingsSection = null;

  function pick(id: Part, e: Event) {
    open = id;
    (e.currentTarget as HTMLElement).closest("main")?.scrollTo({ top: 0 });
  }

  // What each row says now, and its dot.
  const brainModel = $derived(brain ? modelOf(brain) : "");
  const brainModelName = $derived(brain === "anthropic" ? (CLAUDE_MODELS.find(([m]) => m === brainModel)?.[1].split(" — ")[0] ?? brainModel) : brainModel);
  const keyed = $derived(keys.filter((k) => k.source).length);
  const firmware = $derived(store.firmware?.state === "version" ? store.firmware.version : null);
  const via = $derived(store.port?.startsWith("wifi:") ? i18n.t.link.wifi : store.port ? i18n.t.link.usb : "");
  const vs = $derived(voice?.settings);
  const brainProblem = $derived(brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain);
  const pocketSummary = $derived.by(() => {
    const p = store.pocket;
    const tp = i18n.t.pocket;
    if (!p) return { text: tp.unavailable, dot: "" };
    if (p.phase === "off") return { text: tp.phases.off, dot: "" };
    const dot = p.phase === "connected" ? "done" : p.phase === "refused" || p.phase === "no_bluetooth" ? "wrong" : p.phase === "unpaired" ? "" : "think";
    return { text: [tp.phases[p.phase], p.phase === "connected" && p.battery !== null ? `${p.battery}%` : ""].filter(Boolean).join(" · "), dot };
  });
  const meetingProblem = $derived(
    meetings && transcriptionRequirement(meetings.settings.transcribe_by, meetings.model, !!store.info?.google_key)
      ? meetings.settings.transcribe_by === "local"
        ? t.meetingsNoModel
        : i18n.t.meetings.noGoogleKey
      : null,
  );
  const summary = $derived<Record<Part, { text: string; dot: string }>>({
    brain: !store.info?.ready
      ? { text: brainProblem, dot: "wrong" }
      : { text: `${brain ? BRAIN_NAMES[brain] : ""}${brainModelName ? " · " + brainModelName : ""}`, dot: store.info?.limited ? "wait" : "" },
    voice: {
      text: !vs ? "—" : [vs.enabled ? t.listening : t.notListening, vs.live ? i18n.t.board.live : vs.speak ? "" : t.silent].filter(Boolean).join(" · "),
      dot: "",
    },
    keys: { text: keyed ? t.keysOf(keyed, keys.length) : t.noKeys, dot: "" },
    mac: { text: [notch ? t.notchOn : t.notchOff, mcp?.registered ? t.claudeCode : ""].filter(Boolean).join(" · "), dot: "" },
    board:
      store.boardState === "off"
        ? { text: t.boardOff, dot: "" }
        : { text: [store.boardState === "boot" ? i18n.t.link.boot : i18n.t.link.live, via, firmware].filter(Boolean).join(" · "), dot: store.boardState === "boot" ? "think" : "done" },
    meetings: !meetings
      ? { text: "—", dot: "" }
      : meetingProblem
        ? { text: meetingProblem, dot: "wait" }
        : { text: `${t.meetingsWhen[meetings.settings.transcribe]} · ${t.meetingsBy(meetings.settings.summary_by.split(":").pop() ?? "")}`, dot: "" },
    pocket: pocketSummary,
  });
  const loginNeeded = $derived(store.connectors.filter((c) => c.enabled && (c.state === "needs_login" || c.state === "error")).length);
  const connectorsOn = $derived(store.connectors.filter((c) => c.enabled).length);
  const routines = $derived(s?.routines);
  const pageSummary = (p: Page) =>
    p === "connectors" ? (connectorsOn ? t.connectorsOn(connectorsOn) : t.moreNotes.connectors) : p === "routines" || p === "tasks" || p === "spoken" || p === "animations" ? t.moreNotes[p] : "";
  /** A tag at the end of a row: an update for the board, a login for the apps. */
  const badge = (r: Row): { text: string; tone: string } | null =>
    "part" in r && r.part === "board" && store.update ? { text: t.updateBadge, tone: "think" } : "page" in r && r.page === "connectors" && loginNeeded ? { text: t.needsLogin, tone: "wait" } : null;

  // What's wrong right now, each with where it's fixed. Only what stops
  // Yoru from doing something: an update is good news, a tag on its row.
  type Issue = { text: string; tone: "wrong" | "wait"; to: Part | Page };
  const issues = $derived.by<Issue[]>(() => {
    const list: Issue[] = [];
    if (store.info && !store.info.ready) list.push({ text: brainProblem, tone: "wrong", to: "brain" });
    else if (store.info?.limited) list.push({ text: store.info.limited, tone: "wait", to: "brain" });
    if (vs && !vs.live && vs.enabled && voice?.stt_error) list.push({ text: t.issueVoice(voice.stt_error), tone: "wait", to: "voice" });
    if (vs && !vs.live && vs.speak && voice?.tts_error) list.push({ text: t.issueVoice(voice.tts_error), tone: "wait", to: "voice" });
    if (meetingProblem) list.push({ text: meetingProblem, tone: "wait", to: "meetings" });
    if (loginNeeded) list.push({ text: t.issueConnectors(loginNeeded), tone: "wait", to: "connectors" });
    return list;
  });
  const PARTS: Part[] = Object.keys(PART_ICONS) as Part[];
  function goTo(to: Part | Page, e: Event) {
    if ((PARTS as string[]).includes(to)) pick(to as Part, e);
    else store.go(to as Page);
  }
  const okLine = $derived([summary.brain.text, summary.voice.text, summary.board.text].filter((x) => x && x !== "—").join(" · "));

  // The quick settings: his voice in the app's language, and do not disturb.
  const voiceName = $derived.by(() => {
    if (!vs) return "—";
    if (vs.live) return `Gemini Live · ${vs.live}`;
    if (!vs.speak) return t.silent;
    const id = vs.voices[i18n.lang] ?? Object.values(vs.voices).find(Boolean) ?? "";
    // Piper's ids read it_IT-paola-medium: the name is the middle part.
    const piper = /^[a-z]{2}_[A-Z]{2}-([a-z]+)-/.exec(id);
    if (piper) return piper[1][0].toUpperCase() + piper[1].slice(1);
    const note = voice?.models.find((m) => m.id === id)?.note;
    return note?.split(/[,—·]/)[0].trim() || id.replace(/^[^:]+:(voice\/)?/, "") || "—";
  });
  const canTry = $derived(!!vs?.speak && voice?.tts === "ready" && store.boardState === "live");
  let trying = $state(false);
  async function tryVoice() {
    trying = true;
    await run(() => call("test_voice", { board: null, language: i18n.lang }), () => {});
    trying = false;
  }
  const setQuiet = (on: boolean) => store.saveSettings((x) => (x.routines.quiet = on));
</script>

<div class="page">
  {#if open}
    <div class="sub" in:fly={{ y: 10, duration: 260 }}>
      <button class="btn quiet small back" onclick={() => ((open = null), readAll())}>
        <Icon name="chevron-left" size={16} />{t.back}
      </button>
      <div class="sub-head">
        <span class="glyph big"><Icon name={PART_ICONS[open]} size={26} /></span>
        <div>
          <h1>{t.parts[open]}</h1>
          <p class="lede">{t.partLedes[open]}</p>
        </div>
      </div>
      {#if open === "brain"}{@render brainPage()}
      {:else if open === "voice"}<Voice />{@render pageLink("spoken")}
      {:else if open === "keys"}{@render keysPage()}
      {:else if open === "mac"}{@render macPage()}
      {:else if open === "board"}<Board />{@render pageLink("animations")}
      {:else if open === "meetings"}<MeetingSettingsPage />
      {:else}<PocketSettingsPage />{/if}
      {#if error}<p class="error-text note">{error}</p>{/if}
    </div>
  {:else}
    <div class="page-head">
      <div>
        <h1>{t.title}</h1>
      </div>
    </div>

    <!-- What's wrong, or that all is well: Yoru's face says it first. -->
    <section class="status panel" class:bad={issues.length} aria-live="polite">
      <div class="face">
        <YoruBoard board={store.boardState === "off" ? "off" : "live"} size={64} voice={store.voice} gaze={store.gaze} />
      </div>
      <div class="status-text">
        <h2>{issues.length ? t.statusIssues(issues.length) : t.statusOk}</h2>
        {#if !issues.length}<p class="dim">{okLine}</p>{/if}
        {#each issues as issue, i (i)}
          <div class="issue">
            <span class="dot {issue.tone}"></span>
            <span class="issue-text">{issue.text}</span>
            <button class="btn small" class:primary={i === 0} onclick={(e) => goTo(issue.to, e)}>{t.fix}</button>
          </div>
        {/each}
      </div>
    </section>

    <h2 class="section-title">{t.quick}</h2>
    <div class="quick">
      <div class="panel q">
        <button class="q-open" onclick={(e) => pick("voice", e)}>
          <span class="q-label">{t.quickVoice}</span>
          <b>{voiceName}</b>
        </button>
        {#if canTry}
          <button class="btn small" disabled={trying} onclick={tryVoice}><Icon name="play" size={14} />{t.tryVoice}</button>
        {/if}
      </div>
      <div class="panel q">
        <label class="q-open">
          <span class="q-label">{t.quickLanguage}</span>
          <select value={i18n.choice ?? ""} onchange={(e) => i18n.set((e.currentTarget.value || null) as Lang | null)} aria-label={t.quickLanguage}>
            <option value="">{t.system} · {langName(i18n.lang)}</option>
            {#each LANGUAGES as l (l.id)}<option value={l.id}>{l.name}</option>{/each}
          </select>
        </label>
      </div>
      <div class="panel q" class:bad={!store.info?.ready}>
        <button class="q-open" onclick={(e) => pick("brain", e)}>
          <span class="q-label">{t.provider}</span>
          <b class:error-text={!store.info?.ready}>{summary.brain.text}</b>
        </button>
      </div>
      <div class="panel q">
        <button class="q-open" onclick={() => store.go("routines")}>
          <span class="q-label">{t.quickQuiet}</span>
          <b>{routines?.quiet ? `${routines.quiet_from} – ${routines.quiet_to}` : t.quietOff}</b>
        </button>
        {#if routines}<Toggle checked={routines.quiet} label={t.quickQuiet} onchange={setQuiet} />{/if}
      </div>
    </div>

    {#each GROUPS as g (g.id)}
      <h2 class="section-title">{t.groups[g.id]}</h2>
      <nav class="menu panel" aria-label={t.groups[g.id]}>
        {#each g.rows as r ("part" in r ? r.part : r.page)}
          {@const b = badge(r)}
          {#if "part" in r}
            {@const sum = summary[r.part]}
            <button onclick={(e) => pick(r.part, e)}>
              <span class="glyph"><Icon name={PART_ICONS[r.part]} size={18} /></span>
              <span class="words">
                <b>{t.parts[r.part]}{#if sum.dot}<span class="dot {sum.dot}"></span>{/if}</b>
                <span class="says" title={sum.text}>{sum.text}</span>
              </span>
              {#if b}<span class="tag {b.tone}">{b.text}</span>{/if}
              <span class="chev"><Icon name="chevron-right" size={16} /></span>
            </button>
          {:else}
            <button onclick={() => store.go(r.page)}>
              <span class="glyph"><Icon name={PAGE_ICONS[r.page]} size={18} /></span>
              <span class="words">
                <b>{i18n.t.nav[r.page]}</b>
                <span class="says">{pageSummary(r.page)}</span>
              </span>
              {#if b}<span class="tag {b.tone}">{b.text}</span>{/if}
              <span class="chev"><Icon name="chevron-right" size={16} /></span>
            </button>
          {/if}
        {/each}
      </nav>
    {/each}

    <h2 class="section-title">{t.groups.general}</h2>
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.theme}</b><span>{t.lookNote}</span></div>
        <div class="how seg" role="radiogroup" aria-label={t.theme}>
          {#each THEMES as th (th)}
            <button role="radio" aria-checked={look.theme === th} class:on={look.theme === th} onclick={() => look.setTheme(th)}>{t.themes[th]}</button>
          {/each}
        </div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.advancedToggle}</b><span>{t.advancedToggleHint}</span></div>
        <div class="how"><Toggle checked={look.advanced} label={t.advancedToggle} onchange={(on) => look.setAdvanced(on)} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.setupAgain}</b></div>
        <div class="how"><button class="btn small" onclick={setupAgain}>{t.setupRun}</button></div>
      </div>
      {#if look.advanced}
        <div class="setting">
          <div class="what"><b>{t.data}</b><span class="mono">{store.info?.dir ?? "—"}</span></div>
        </div>
      {/if}
    </section>
  {/if}
</div>

<!-- A page of the app that belongs with this one: Spoken under Voice, Animations under the board. -->
{#snippet pageLink(p: "spoken" | "animations")}
  <nav class="menu panel link" aria-label={i18n.t.nav[p]}>
    <button onclick={() => store.go(p)}>
      <span class="glyph"><Icon name={PAGE_ICONS[p]} size={18} /></span>
      <span class="words"><b>{i18n.t.nav[p]}</b><span class="says">{t.moreNotes[p]}</span></span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
  </nav>
{/snippet}

<!--
  The brain in use, with its key where a banner's "Fix" leads; then the
  guided change of service; the brain's own tiles and model are advanced.
-->
{#snippet brainPage()}
  {#if s}
    <section class="panel active">
      <div class="setting">
        <div class="what"><b>{t.inUse}</b><span class:error-text={!store.info?.ready}>{summary.brain.text}</span></div>
      </div>
      {#if store.info?.ready && store.info?.limited}<p class="error-text note">{store.info.limited}</p>{/if}
      {#if brain && brain !== "ollama"}
        {@const k = keyOf(brain)}
        {#if k}
          <div class="block">
            <div class="what"><b>{t.keyOf(k.name)}</b><span class="dim">{k.hint}</span></div>
            <KeyField provider={k.id} source={k.source} placeholder={k.placeholder} onchange={() => (store.refreshInfo(), readAll())} />
          </div>
        {/if}
      {/if}
      {#if brain === "ollama"}{@render modelRow("ollama")}{/if}
      {#if brain === "anthropic"}
        <div class="setting">
          <div class="what"><b>{t.webSearch}</b></div>
          <div class="how"><Toggle checked={s.web_search} label={t.webSearch} onchange={(on) => store.saveSettings((x) => (x.web_search = on))} /></div>
        </div>
      {/if}
    </section>

    <h2 class="section-title">{t.changeService}</h2>
    <p class="dim small">{t.changeServiceHint}</p>
    <section class="panel wizard"><Profiles onchange={readAll} /></section>

    {#if look.advanced}
      <h2 class="section-title">{t.advanced}</h2>
      <section class="tiles" aria-label={t.provider}>
        {#each BRAINS as b (b)}
          <button class="tile" class:on={brain === b} onclick={() => store.saveSettings((x) => (x.provider = b))}>
            <b>{BRAIN_NAMES[b]}</b>
            <span>{t.brainNotes[b]}</span>
            {#if brain === b && store.info?.ready}<span class="tag">{t.inUse}</span>{/if}
          </button>
        {/each}
      </section>
      {#if brain && brain !== "ollama"}
        <section class="panel">
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
          {:else}
            <p class="dim small">{t.groqNote}</p>
          {/if}
        </section>
      {/if}
    {/if}
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

<!-- Yoru on this Mac: its permissions, the notch, and Claude Code through his MCP server. -->
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
  {#if mcp?.command && look.advanced}
    <h2 class="section-title">{t.advanced}</h2>
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.mcpCommand}</b><span class="mono">{mcp.command} {mcp.args.join(" ")}</span></div>
      </div>
    </section>
  {/if}
{/snippet}


<style>
  /* What's wrong, or all is well. */
  .status {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 16px 20px;
    margin-bottom: 8px;
  }
  .status.bad {
    border-color: color-mix(in srgb, var(--wait) 45%, transparent);
    background: color-mix(in srgb, var(--wait) 6%, var(--ink));
  }
  .face {
    flex-shrink: 0;
  }
  .status-text {
    display: grid;
    gap: 6px;
    flex: 1;
    min-width: 0;
  }
  .status-text h2 {
    font: 600 var(--fs-head)/1.3 var(--display);
  }
  .issue {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .issue-text {
    flex: 1;
    min-width: 0;
  }

  /* The quick settings: two by two. */
  .quick {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  .q {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
  }
  .q.bad {
    border-color: color-mix(in srgb, var(--wrong) 40%, transparent);
  }
  .q-open {
    display: grid;
    gap: 4px;
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .q-open b {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .q-open select {
    justify-self: stretch;
    width: 100%;
    min-width: 0;
  }
  .q-label {
    color: var(--haze);
    font-size: var(--fs-meta);
  }

  /* The groups' rows. */
  .menu {
    display: grid;
    padding: 4px;
  }
  .menu.link {
    margin-top: 22px;
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
    background: var(--hover);
    outline: none;
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--ink-2);
    color: var(--haze);
  }
  .glyph.big {
    width: 52px;
    height: 52px;
    color: var(--think);
    background: color-mix(in srgb, var(--think) 14%, transparent);
  }
  .words {
    display: grid;
    gap: 2px;
    min-width: 0;
    flex: 1;
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--haze);
    font-size: var(--fs-meta);
    line-height: 1.35;
  }
  .tag.wait {
    color: var(--wait);
    border-color: color-mix(in srgb, var(--wait) 40%, transparent);
  }
  .chev {
    flex-shrink: 0;
    color: var(--dusk);
    transition: transform 200ms;
  }
  .menu button:hover .chev {
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
    margin: 14px 0 22px;
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
  .wizard {
    padding: 16px 18px;
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
  .note {
    margin-top: 12px;
  }

  .block {
    display: grid;
    gap: 12px;
    padding: 6px 0 16px;
  }
  .setting + .block,
  .block + .setting,
  .block + .block {
    border-top: 1px solid var(--rim);
  }
  .block + .block,
  .setting + .block {
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
</style>
