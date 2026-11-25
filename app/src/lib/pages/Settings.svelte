<!--
  Settings, sorted by what you want done rather than by how Yoru is built.
  Rows in four groups, each with what it holds now and a dot when
  something is wrong there: Yoru,
  Devices, Accounts and services, General. Every setting lives in one place
  only: every model and key, with whether Yoru listens and answers aloud
  and how meetings are transcribed and summarized, in Intelligence
  (ui/Intelligence.svelte). A row opens its page with the everyday choices; models, ports and
  MCP show only with "Show advanced options", one switch for every page.
-->
<script lang="ts">
  import { onWifi } from "../port";
  import { look, THEMES } from "../look.svelte";
  import { fly } from "svelte/transition";
  import { call, why } from "../api";
  import { i18n, LANGUAGES, type Lang } from "../i18n/index.svelte";
  import { store, type Page } from "../store.svelte";
  import type { Brain, McpInfo } from "../types";
  import Board from "../ui/Board.svelte";
  import PermissionList from "../ui/PermissionList.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import PocketSettingsPage from "../ui/PocketSettings.svelte";
  import ICloudSettingsPage from "../ui/ICloudSettings.svelte";
  import Intelligence, { type Job } from "../ui/Intelligence.svelte";
  import { PAGE_ICONS } from "../icons/pages";
  import Icon from "../ui/Icon.svelte";
  import type { IconName } from "../icons/icons";

  const t = $derived(i18n.t.settings);
  const s = $derived(store.info?.settings);
  /** Claude's models, by what they're like; the ids are in the option's value. */
  const CLAUDE_MODELS = $derived([
    ["claude-opus-5-5", t.claudeModels.opus],
    ["claude-sonnet-5-5", t.claudeModels.sonnet],
    ["claude-haiku-5-5", t.claudeModels.haiku],
  ] as const);
  const brain = $derived(store.info?.brain ?? null);
  const BRAIN_NAMES: Record<Brain, string> = { anthropic: "Claude", openai: "OpenAI", google: "Gemini", groq: "Groq", ollama: "Ollama" };
  const langName = (id: Lang) => LANGUAGES.find((l) => l.id === id)?.name ?? id;

  // Each brain's model, where it's kept, and the models it offers.
  const MODEL_FIELD = { anthropic: "model", openai: "openai_model", google: "google_model", groq: "groq_model", ollama: "ollama_model" } as const;
  const modelOf = (b: Brain) => (s ? s[MODEL_FIELD[b]] : "");


  let mcp = $state<McpInfo | null>(null);
  let notch = $state(false);
  let error = $state("");

  async function readAll() {
    mcp = await call<McpInfo>("mcp_info").catch(() => null);
    notch = (await call<{ notch?: boolean }>("status").catch(() => null))?.notch ?? false;
  }
  $effect(() => {
    void readAll();
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
  type Part = "brain" | "mac" | "board" | "pocket" | "icloud";
  const PART_ICONS: Record<Part, IconName> = { brain: "sparkles", mac: "laptop", board: "cpu", pocket: "mic", icloud: "cloud" };
  type Row = { part: Part } | { page: Page };
  const GROUPS: { id: string; rows: Row[] }[] = [
    { id: "yoru", rows: [{ part: "brain" }, { page: "routines" }, { page: "tasks" }] },
    { id: "devices", rows: [{ part: "board" }, { part: "pocket" }, { part: "mac" }] },
    { id: "accounts", rows: [{ page: "connectors" }, { part: "icloud" }] },
  ];

  let open = $state<Part | null>(store.settingsSection ? "brain" : null);
  /** Intelligence's row to open with it: the voice tile opens Speaks, a meeting's problem Transcribes. */
  let row = $state<Job | null>(store.settingsSection);
  store.settingsSection = null;
  // The sidebar's iCloud row, also while Settings is already open.
  $effect(() => {
    if (!store.settingsPart) return;
    open = store.settingsPart;
    row = null;
    store.settingsPart = null;
  });
  function pick(id: Part, e: Event, job: Job | null = null) {
    open = id;
    row = job;
    (e.currentTarget as HTMLElement).closest("main")?.scrollTo({ top: 0 });
  }

  // What each row says now, and its dot.
  const brainModel = $derived(brain ? modelOf(brain) : "");
  const brainModelName = $derived(brain === "anthropic" ? (CLAUDE_MODELS.find(([m]) => m === brainModel)?.[1].split(" — ")[0] ?? brainModel) : brainModel);
  const firmware = $derived(store.firmware?.state === "version" ? store.firmware.version : null);
  const via = $derived(onWifi(store.port) ? i18n.t.link.wifi : store.port ? i18n.t.link.usb : "");
  const brainProblem = $derived(brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain);
  const pocketSummary = $derived.by(() => {
    const p = store.pocket;
    const tp = i18n.t.pocket;
    if (!p) return { text: tp.unavailable, dot: "" };
    if (p.phase === "off") return { text: tp.phases.off, dot: "" };
    const dot = p.phase === "connected" ? "done" : p.phase === "refused" || p.phase === "no_bluetooth" ? "wrong" : p.phase === "unpaired" ? "" : "think";
    return { text: [tp.phases[p.phase], p.phase === "connected" && p.battery !== null ? `${p.battery}%` : ""].filter(Boolean).join(" · "), dot };
  });
  const readICloud = () => store.refreshICloud();
  const icloudOn = $derived(store.icloudOn);
  $effect(() => {
    void readICloud();
  });
  const icloudSummary = $derived.by(() => {
    const ti = i18n.t.icloud;
    if (!icloudOn) return { text: ti.off, dot: "" };
    const c = store.icloud;
    if (!c) return { text: ti.accounts.unknown, dot: "think" };
    if (c.errors.length && c.last_error > Math.max(c.last_sent, c.last_fetched)) return { text: c.errors[c.errors.length - 1], dot: "wrong" };
    if (c.account !== "available") return { text: ti.accounts[c.account] ?? c.account, dot: "wait" };
    return { text: [ti.accounts.available, c.is_doer ? ti.doerHere : ""].filter(Boolean).join(" · "), dot: "done" };
  });
  const summary = $derived<Record<Part, { text: string; dot: string }>>({
    brain: !store.info?.ready
      ? { text: brainProblem, dot: "wrong" }
      : { text: `${brain ? BRAIN_NAMES[brain] : ""}${brainModelName ? " · " + brainModelName : ""}`, dot: store.info?.limited ? "wait" : "" },
    mac: { text: [notch ? t.notchOn : t.notchOff, mcp?.registered ? t.claudeCode : ""].filter(Boolean).join(" · "), dot: "" },
    board:
      store.boardState === "off"
        ? { text: t.boardOff, dot: "" }
        : { text: [store.boardState === "boot" ? i18n.t.link.boot : i18n.t.link.live, via, firmware].filter(Boolean).join(" · "), dot: store.boardState === "boot" ? "think" : "done" },
    pocket: pocketSummary,
    icloud: icloudSummary,
  });
  const loginNeeded = $derived(store.connectors.filter((c) => c.enabled && (c.state === "needs_login" || c.state === "error")).length);
  const connectorsOn = $derived(store.connectors.filter((c) => c.enabled).length);
  const pageSummary = (p: Page) =>
    p === "connectors" ? (connectorsOn ? t.connectorsOn(connectorsOn) : t.moreNotes.connectors) : p === "routines" || p === "tasks" || p === "spoken" || p === "animations" ? t.moreNotes[p] : "";
  /** A tag at the end of a row: an update for the board, a login for the apps. */
  const badge = (r: Row): { text: string; tone: string } | null =>
    "part" in r && r.part === "board" && store.update ? { text: t.updateBadge, tone: "think" } : "page" in r && r.page === "connectors" && loginNeeded ? { text: t.needsLogin, tone: "wait" } : null;
</script>

<div class="page">
  {#if open}
    <div class="sub" in:fly={{ y: 10, duration: 260 }}>
      <button class="btn quiet small back" onclick={() => ((open = null), readAll(), readICloud())}>
        <Icon name="chevron-left" size={16} />{t.back}
      </button>
      <div class="sub-head">
        <span class="glyph big"><Icon name={PART_ICONS[open]} size={26} /></span>
        <div>
          <h1>{t.parts[open]}</h1>
          <p class="lede">{t.partLedes[open]}</p>
        </div>
      </div>
      {#if open === "brain"}<Intelligence {row} />{@render pageLink("spoken")}
      {:else if open === "mac"}{@render macPage()}
      {:else if open === "board"}<Board />{@render pageLink("animations")}
      {:else if open === "pocket"}<PocketSettingsPage />
      {:else}<ICloudSettingsPage />{/if}
      {#if error}<p class="error-text note">{error}</p>{/if}
    </div>
  {:else}
    <div class="page-head">
      <div>
        <h1>{t.title}</h1>
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
        <div class="what"><b>{t.quickLanguage}</b></div>
        <div class="how">
          <select value={i18n.choice ?? ""} onchange={(e) => i18n.set((e.currentTarget.value || null) as Lang | null)} aria-label={t.quickLanguage}>
            <option value="">{t.system} · {langName(i18n.lang)}</option>
            {#each LANGUAGES as l (l.id)}<option value={l.id}>{l.name}</option>{/each}
          </select>
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
