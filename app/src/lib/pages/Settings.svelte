<!--
  Yoru's brain (Groq, OpenAI, Claude or Ollama, their keys and models, how
  hard Claude thinks), other providers' keys, the app's language, and Claude Code's integration.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n, LANGUAGES, type Lang } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Brain, ClaudeAlertsInfo, ClaudeLink, Effort, McpInfo, VoiceInfo } from "../types";
  import KeyField from "../ui/KeyField.svelte";
  import Toggle from "../ui/Toggle.svelte";

  const t = $derived(i18n.t.settings);
  const s = $derived(store.info?.settings);
  const EFFORTS: Effort[] = ["low", "medium", "high", "xhigh", "max"];
  const CLAUDE_MODELS = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
  const brain = $derived(store.info?.brain ?? null);
  const BRAIN_NAMES: Record<Brain, string> = { anthropic: "Claude", openai: "OpenAI", groq: "Groq", ollama: "Ollama" };

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

  // The brains' keys (Groq's and OpenAI's serve speech to text too) are in the brain's section.
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
</script>

<div class="page">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  <h2 class="section-title">{t.brain}</h2>
  <section class="panel">
    {#if s}
      <div class="setting">
        <div class="what"><b>{t.provider}</b>{#if !store.info?.ready}<span class="error-text">{brain === "ollama" ? t.ollamaNoModel : brain ? t.brainNoKey(BRAIN_NAMES[brain]) : t.noBrain}</span>{:else if store.info?.limited}<span class="error-text">{store.info.limited}</span>{/if}</div>
        <div class="how">
          <select value={s.provider ?? ""} onchange={(e) => store.saveSettings((x) => (x.provider = (e.currentTarget.value || null) as Brain | null))} aria-label={t.provider}>
            <option value="">{t.providers.auto}</option>
            <option value="groq">{t.providers.groq}</option>
            <option value="openai">{t.providers.openai}</option>
            <option value="anthropic">{t.providers.anthropic}</option>
            <option value="ollama">{t.providers.ollama}</option>
          </select>
        </div>
      </div>
    {/if}
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
    {/if}
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
    {/if}
  </section>

  {#if others.length}
    <h2 class="section-title">{t.otherKeys}</h2>
    <section class="panel">
      <p class="sub">{t.otherKeysHint}</p>
      {#each others as p (p.id)}
        <div class="block">
          <div class="what"><b>{p.name}</b><span class="dim">{p.note}</span></div>
          <KeyField provider={p.id} source={p.key} onchange={readAll} />
        </div>
      {/each}
    </section>
  {/if}

  <h2 class="section-title">{t.language}</h2>
  <section class="panel">
    <div class="setting">
      <div class="what"><b>{t.appLanguage}</b></div>
      <div class="how">
        <select value={i18n.choice ?? ""} onchange={(e) => i18n.set((e.currentTarget.value || null) as Lang | null)} aria-label={t.appLanguage}>
          <option value="">{t.system}</option>
          {#each LANGUAGES as l (l.id)}<option value={l.id}>{l.name}</option>{/each}
        </select>
      </div>
    </div>
  </section>

  <h2 class="section-title">{t.claude}</h2>
  <section class="panel">
    <p class="sub">{t.claudeHint}</p>
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
    {#if alerts}
      <div class="setting">
        <div class="what"><b>{t.approvals}</b><span>{t.approvalsHint}</span></div>
        <div class="how"><Toggle checked={alerts.settings.approvals} label={t.approvals} onchange={(on) => setAlerts({ approvals: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.alertsSpeak}</b></div>
        <div class="how"><Toggle checked={alerts.settings.speak} label={t.alertsSpeak} onchange={(on) => setAlerts({ speak: on })} /></div>
      </div>
    {/if}
    {#if error}<p class="error-text">{error}</p>{/if}
  </section>

  <h2 class="section-title">{t.about}</h2>
  <section class="panel">
    <div class="setting">
      <div class="what"><b>{t.data}</b><span class="mono">{store.info?.dir ?? "—"}</span></div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.setupAgain}</b></div>
      <div class="how"><button class="btn small" onclick={setupAgain}>{t.setupRun}</button></div>
    </div>
  </section>
</div>

<style>
  .block {
    display: grid;
    gap: 12px;
    padding: 6px 0 16px;
  }
  .block + .block {
    border-top: 1px solid var(--rim);
    padding-top: 16px;
  }
  .block + .setting {
    border-top: 1px solid var(--rim);
  }
  .setting + .block {
    border-top: 1px solid var(--rim);
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
  .sub {
    margin-bottom: 8px;
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
</style>
