<!--
  Claude Code's sessions, the way coucou follows its agents: each one's
  project, what it's doing and its last steps, waiting ones in orange,
  working in blue, done in green. A permission request waits on top with
  Allow and Deny; the same goes to Yoru's screen. Fed by the hooks.
-->
<script lang="ts">
  import Pill from "./Pill.svelte";
  import { monitor, type AgentSession, type AgentState, type Approval, type StepKind } from "../monitor.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { settingsData } from "../settings/data.svelte";
  import { settingsNav } from "../settings/nav.svelte";

  /** Sessions shown, most recently active first. */
  const SHOWN = 4;
  /** Steps shown for each. */
  const STEPS = 3;

  const t = $derived(i18n.t.yoru.agents);
  const view = $derived(monitor.agents);
  const sessions = $derived(view.sessions.slice(0, SHOWN));
  const live = $derived(view.sessions.filter((s) => s.state === "work" || s.state === "wait").length);
  const waiting = $derived(Math.max(view.approvals.length, view.sessions.filter((s) => s.state === "wait").length));
  const alerts = $derived(settingsData.alerts);
  const answerHere = $derived(alerts?.settings.approvals ?? false);

  const COLORS: Record<AgentState, string> = { work: "#4aa3ff", wait: "#ff9f43", done: "#40e080", idle: "#8a8fa0" };
  const GLYPHS: Record<StepKind, string> = { read: "◎", edit: "✎", write: "✚", run: "›_", search: "⌕", web: "◍", agent: "◇", ask: "?", other: "•" };

  const accent = $derived(waiting ? COLORS.wait : live ? COLORS.work : sessions[0]?.state === "done" ? COLORS.done : COLORS.idle);
  const chip = $derived(waiting ? t.waiting(waiting) : live ? t.live(live) : null);

  /** Seconds left before Claude Code asks in its terminal instead. */
  const left = (a: Approval) => Math.max(0, Math.round((a.expires - monitor.now) / 1000));
  const ago = (ms: number) => t.ago(Math.max(0, Math.round((monitor.now - ms) / 1000)));
  const steps = (s: AgentSession) => s.steps.slice(-STEPS).reverse();

  let answering = $state<string | null>(null);
  async function answer(a: Approval, allow: boolean) {
    answering = a.id;
    try {
      await monitor.approve(a.id, allow);
    } finally {
      answering = null;
    }
  }
</script>

<Pill title={t.title} {accent} {chip} chipLive={live > 0}>
  {#each view.approvals as a (a.id)}
    <div class="ask">
      <div class="ask-head">
        <span class="project">{a.project ?? "Claude Code"}</span>
        <span class="left">{t.terminal(left(a))}</span>
      </div>
      <span class="asks">{a.kind === "run" ? t.asks : a.tool}</span>
      <code>{a.text || a.tool}</code>
      {#if a.detail}<span class="detail">{a.detail}</span>{/if}
      <div class="buttons">
        <button class="deny" disabled={answering === a.id} onclick={() => answer(a, false)}>{t.deny}</button>
        <button class="allow" disabled={answering === a.id} onclick={() => answer(a, true)}>{t.allow}</button>
      </div>
    </div>
  {/each}

  {#if sessions.length}
    <ul class="sessions">
      {#each sessions as s (s.id)}
        <li style:--state={COLORS[s.state]}>
          <div class="head">
            <i class="dot" class:live={s.state === "work" || s.state === "wait"}></i>
            <span class="name">{s.project ?? s.id.slice(0, 8)}</span>
            <span class="state">{t.states[s.state]}</span>
            <span class="ago">{ago(s.updated)}</span>
          </div>
          {#if s.state === "wait" && s.waiting}
            <p class="task waiting">{s.waiting}</p>
          {:else if s.task}
            <p class="task">{s.task}</p>
          {/if}
          {#if s.steps.length}
            <ol class="steps">
              {#each steps(s) as step, i (step.at + step.text + i)}
                <li class:failed={step.ok === false} class:running={step.ok === undefined}>
                  <span class="glyph" title={t.kinds[step.kind]}>{GLYPHS[step.kind]}</span>
                  <span class="text">{step.text || step.tool}</span>
                  {#if step.added}<b class="add">+{step.added}</b>{/if}
                  {#if step.removed}<b class="del">−{step.removed}</b>{/if}
                </li>
              {/each}
            </ol>
          {/if}
        </li>
      {/each}
    </ul>
  {:else}
    <p class="empty">{t.none}</p>
  {/if}

  {#if alerts && !alerts.hooks.connected}
    <button class="link" onclick={() => settingsNav.go({ tab: "claude", id: "alerts" })}>{t.hooks}</button>
  {:else if alerts && !answerHere}
    <div class="off">
      <span>{t.answerOff}</span>
      <button class="link" onclick={() => settingsData.setAlerts({ approvals: true })}>{t.answerHere}</button>
    </div>
  {/if}
</Pill>

<style>
  .ask {
    display: grid;
    gap: 6px;
    padding: 11px 12px 12px;
    border-radius: 14px;
    background: color-mix(in srgb, #ff9f43 9%, transparent);
    border: 1px solid color-mix(in srgb, #ff9f43 32%, transparent);
    animation: arrive 320ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .ask-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .project {
    font: 600 13px/1.2 var(--sans);
    color: var(--text);
  }
  .left {
    margin-left: auto;
    font: 500 10.5px/1 var(--sans);
    color: var(--faint);
    white-space: nowrap;
  }
  .asks {
    font: 550 10.5px/1 var(--sans);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #ffb36b;
  }
  code {
    display: block;
    padding: 7px 9px;
    border-radius: 9px;
    background: rgba(0, 0, 0, 0.35);
    font: 500 12px/1.35 var(--mono);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .detail {
    font: 450 11.5px/1.35 var(--sans);
    color: var(--dim);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 2px;
  }
  .buttons button {
    height: 28px;
    padding: 0 14px;
    border-radius: 999px;
    font: 600 12px/1 var(--sans);
    cursor: pointer;
    transition:
      transform 120ms,
      filter 120ms;
  }
  .buttons button:active {
    transform: scale(0.96);
  }
  .buttons button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .deny {
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.04);
    color: var(--dim);
  }
  .deny:hover {
    color: var(--text);
  }
  .allow {
    border: 0;
    background: #ff9f43;
    color: #1d1206;
  }
  .allow:hover {
    filter: brightness(1.08);
  }

  .sessions {
    display: grid;
    gap: 10px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .sessions > li {
    display: grid;
    gap: 5px;
    padding-left: 10px;
    border-left: 2px solid color-mix(in srgb, var(--state) 55%, transparent);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }
  .dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--state);
  }
  .dot.live {
    box-shadow: 0 0 8px var(--state);
    animation: breathe 1.6s ease-in-out infinite;
  }
  .name {
    font: 600 13px/1.2 var(--sans);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .state {
    flex: none;
    font: 550 11px/1 var(--sans);
    color: color-mix(in srgb, var(--state) 75%, var(--text));
  }
  .ago {
    margin-left: auto;
    flex: none;
    font: 500 10.5px/1 var(--mono);
    color: var(--faint);
  }
  .task {
    margin: 0;
    font: 450 12px/1.35 var(--sans);
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .task.waiting {
    color: #ffb36b;
  }
  .steps {
    display: grid;
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .steps li {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    font: 500 11.5px/1.3 var(--mono);
    color: var(--dim);
    animation: arrive 260ms ease-out;
  }
  .steps li:not(:first-child) {
    opacity: 0.62;
  }
  .glyph {
    flex: none;
    width: 16px;
    text-align: center;
    color: var(--faint);
  }
  .steps li.running .glyph {
    color: var(--state);
  }
  .steps li.failed .glyph,
  .steps li.failed .text {
    color: #ff6b6b;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .add,
  .del {
    flex: none;
    font: 600 11px/1 var(--mono);
  }
  .add {
    color: #40e080;
  }
  .del {
    color: #ff6b6b;
  }
  .empty,
  .off span {
    margin: 0;
    font: 450 12px/1.45 var(--sans);
    color: var(--dim);
  }
  .off {
    display: grid;
    gap: 6px;
  }
  .link {
    justify-self: start;
    align-self: flex-start;
    border: 0;
    padding: 0;
    background: none;
    color: var(--dim);
    font: 500 12px/1 var(--sans);
    text-decoration: underline;
    text-decoration-color: rgba(255, 255, 255, 0.2);
    text-underline-offset: 3px;
    cursor: pointer;
  }
  .link:hover {
    color: var(--text);
  }
  @keyframes breathe {
    50% {
      opacity: 0.35;
    }
  }
  @keyframes arrive {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
