<!--
  The Home's right side, which folds away: the coding agents at work, the
  tasks Yoru does on his own and his routines (the morning recap, the
  watchers), each a line with what it is and how it stands. A line opens
  its detail in a dialog in the middle of the window.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ActiveAgent, TaskEntry, WatchEntry } from "../types";
  import Dialog from "./Dialog.svelte";
  import { ago } from "./when";

  let { open = $bindable(true) }: { open?: boolean } = $props();

  const t = $derived(i18n.t.home);
  const ta = $derived(i18n.t.agents);
  const tt = $derived(i18n.t.tasks);
  const AGENT_NAMES: Record<ActiveAgent["agent"], string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode" };
  const folder = (path: string) => path.split("/").filter(Boolean).pop() ?? path;
  const when = (at: number) => `${i18n.date(at, { weekday: "short", day: "numeric", month: "short" })} ${i18n.time(at)}`;

  const agentDot = (a: ActiveAgent) =>
    a.state === "working" ? "think" : a.state === "done" ? "done" : a.state === "needs_you" ? "wait" : a.state === "failed" ? "wrong" : "";
  const agentKey = (a: ActiveAgent) => `${a.agent}:${a.run ?? a.session}`;

  // Tasks: the running ones first, then the next due; the finished one-offs last.
  const tasks = $derived(
    [...store.tasks]
      .filter((e) => e.task.enabled || e.running || e.state.last_run)
      .sort((a, b) => Number(b.running) - Number(a.running) || (a.next ?? Infinity) - (b.next ?? Infinity) || b.state.last_run - a.state.last_run),
  );
  const taskDot = (e: TaskEntry) => (e.running ? "think" : e.state.last_error ? "wrong" : e.next ? "done" : "");
  const taskLine = (e: TaskEntry) =>
    e.running ? tt.running : e.next ? tt.next(when(e.next)) : e.state.last_run ? t.ran(ago(e.state.last_run, store.now)) : t.notYet;

  const routines = $derived(store.info?.settings.routines ?? null);
  const watchers = $derived(store.watchers.filter((w) => w.watcher.enabled));
  const watcherDot = (w: WatchEntry) => (w.state.last_error ? "wrong" : w.state.last_run ? "done" : "");
  const watcherLine = (w: WatchEntry) =>
    `${i18n.t.routines.every(w.watcher.every_min)} · ${w.state.last_run ? t.ran(ago(w.state.last_run, store.now)) : t.notYet}`;
  const recapLine = $derived(routines ? (routines.recap_weekdays_only ? t.weekdaysAt(routines.recap_at) : t.daily(routines.recap_at)) : "");

  // The detail open in the dialog, looked up again so it stays current.
  type Detail = { kind: "agent"; key: string } | { kind: "task"; id: string } | { kind: "watcher"; id: string } | { kind: "recap" };
  let detail = $state<Detail | null>(null);
  let busy = $state(false);
  let error = $state("");
  const show = (d: Detail) => ((detail = d), (error = ""));
  const agent = $derived(detail?.kind === "agent" ? store.agents.find((a) => agentKey(a) === (detail as { key: string }).key) : undefined);
  const task = $derived(detail?.kind === "task" ? store.tasks.find((e) => e.task.id === (detail as { id: string }).id) : undefined);
  const watcher = $derived(detail?.kind === "watcher" ? store.watchers.find((w) => w.watcher.id === (detail as { id: string }).id) : undefined);

  async function act(job: () => Promise<unknown>) {
    busy = true;
    error = "";
    try {
      await job();
    } catch (e) {
      error = why(e);
    } finally {
      busy = false;
    }
  }
  function goTo(page: "tasks" | "routines") {
    detail = null;
    store.go(page);
  }
</script>

<aside class="activity" class:open aria-label={t.panel}>
  <header>
    {#if open}<h2>{t.panel}</h2>{/if}
    <button class="fold" onclick={() => (open = !open)} aria-label={open ? t.hidePanel : t.showPanel} title={open ? t.hidePanel : t.showPanel}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3.5" y="5" width="17" height="14" rx="2.5" /><path d="M14.5 5v14" />{#if open}<path d="M8.5 10l2 2-2 2" />{:else}<path d="M10.5 10l-2 2 2 2" />{/if}</svg>
    </button>
  </header>

  {#if open}
    <div class="scroll">
      <section>
        <h3 class="section-title">{ta.title}{#if store.working}<span class="n">{store.working}</span>{/if}</h3>
        {#if store.agents.length}
          <ul>
            {#each store.agents.slice(0, 6) as a (agentKey(a))}
              <li>
                <button onclick={() => show({ kind: "agent", key: agentKey(a) })}>
                  <span class="dot {agentDot(a)}"></span>
                  <span class="words">
                    <b>{a.title}</b>
                    <span>{AGENT_NAMES[a.agent]} · {folder(a.project)} · {a.state === "working" ? ago(a.updated, store.now) : ta.states[a.state]}</span>
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="none">{ta.none}</p>
        {/if}
      </section>

      <section>
        <h3 class="section-title">{t.tasks}</h3>
        {#if tasks.length}
          <ul>
            {#each tasks.slice(0, 6) as e (e.task.id)}
              <li>
                <button onclick={() => show({ kind: "task", id: e.task.id })}>
                  <span class="dot {taskDot(e)}"></span>
                  <span class="words"><b>{e.task.name}</b><span>{taskLine(e)}</span></span>
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="none">{t.noTasks}</p>
        {/if}
      </section>

      <section>
        <h3 class="section-title">{t.routines}</h3>
        {#if routines?.recap || watchers.length}
          <ul>
            {#if routines?.recap}
              <li>
                <button onclick={() => show({ kind: "recap" })}>
                  <span class="dot done"></span>
                  <span class="words"><b>{t.recapName}</b><span>{recapLine}</span></span>
                </button>
              </li>
            {/if}
            {#each watchers.slice(0, 6) as w (w.watcher.id)}
              <li>
                <button onclick={() => show({ kind: "watcher", id: w.watcher.id })}>
                  <span class="dot {watcherDot(w)}"></span>
                  <span class="words"><b>{w.watcher.name}</b><span>{watcherLine(w)}</span></span>
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="none">{t.noRoutines}</p>
        {/if}
      </section>
    </div>
  {/if}
</aside>

{#if detail?.kind === "agent" && agent}
  {@const a = agent}
  <Dialog title={a.title} onclose={() => (detail = null)}>
    <dl>
      <dt>{t.agent}</dt><dd>{AGENT_NAMES[a.agent]}{#if a.run !== null} · {ta.byYoru}{/if}</dd>
      <dt>{t.project}</dt><dd class="mono">{a.project}</dd>
      <dt>{t.state}</dt><dd><span class="dot {agentDot(a)}"></span> {ta.states[a.state]}</dd>
      {#if a.now}<dt>{t.step}</dt><dd>{a.now}</dd>{/if}
      <dt>{t.updated}</dt><dd>{ago(a.updated, store.now)}</dd>
    </dl>
    {#snippet actions()}
      {#if a.run !== null && a.state === "working"}
        <button class="btn" disabled={busy} onclick={() => act(() => store.stopAgent(a.run!))}>{ta.stop}</button>
      {/if}
      <button class="btn primary" onclick={() => (detail = null)}>{t.close}</button>
    {/snippet}
  </Dialog>
{:else if detail?.kind === "task" && task}
  {@const e = task}
  <Dialog title={e.task.name} onclose={() => (detail = null)}>
    <dl>
      <dt>{t.what}</dt><dd class="long">{e.task.what}</dd>
      <dt>{t.when}</dt><dd>{tt.schedule(e.task)} · {tt.actingTag[e.task.acting]}{#if !e.task.enabled} · {tt.off}{/if}</dd>
      {#if e.next}<dt>{t.next}</dt><dd>{when(e.next)}</dd>{/if}
      <dt>{t.last}</dt><dd>{e.running ? tt.running : e.state.last_run ? when(e.state.last_run) : tt.never}</dd>
      {#if e.state.last_error}
        <dt>{t.error}</dt><dd class="long error-text">{e.state.last_error}</dd>
      {:else}
        <dt>{t.result}</dt><dd class="long">{e.state.last_result || t.noResult}</dd>
      {/if}
    </dl>
    {#if error}<p class="error-text">{error}</p>{/if}
    {#snippet actions()}
      <button class="btn quiet" onclick={() => goTo("tasks")}>{t.open(i18n.t.nav.tasks)}</button>
      <button class="btn primary" disabled={busy || e.running} onclick={() => act(() => store.runTask(e.task.id))}>{e.running ? tt.running : tt.runNow}</button>
    {/snippet}
  </Dialog>
{:else if detail?.kind === "watcher" && watcher}
  {@const w = watcher}
  <Dialog title={w.watcher.name} onclose={() => (detail = null)}>
    <dl>
      <dt>{t.what}</dt><dd class="long">{w.watcher.kind === "task" ? w.watcher.task : w.watcher.tool}</dd>
      {#if w.watcher.instructions}<dt>{t.instructions}</dt><dd class="long">{w.watcher.instructions}</dd>{/if}
      <dt>{t.every}</dt><dd>{i18n.t.routines.every(w.watcher.every_min)}</dd>
      <dt>{t.last}</dt><dd>{w.state.last_run ? `${when(w.state.last_run)} · ${t.told(w.state.told)}` : t.notYet}</dd>
      {#if w.state.last_error}<dt>{t.error}</dt><dd class="long error-text">{w.state.last_error}</dd>{/if}
    </dl>
    {#if error}<p class="error-text">{error}</p>{/if}
    {#snippet actions()}
      <button class="btn quiet" onclick={() => goTo("routines")}>{t.open(i18n.t.nav.routines)}</button>
      <button class="btn primary" disabled={busy} onclick={() => act(() => call("run_watcher", { id: w.watcher.id }).then(() => store.refreshWatchers()))}>{i18n.t.routines.runNow}</button>
    {/snippet}
  </Dialog>
{:else if detail?.kind === "recap" && routines}
  <Dialog title={t.recapName} onclose={() => (detail = null)}>
    <dl>
      <dt>{t.when}</dt><dd>{recapLine}</dd>
      <dt>{t.what}</dt><dd class="long">{routines.recap_task || i18n.t.routines.recapHint}</dd>
    </dl>
    {#if error}<p class="error-text">{error}</p>{/if}
    {#snippet actions()}
      <button class="btn quiet" onclick={() => goTo("routines")}>{t.open(i18n.t.nav.routines)}</button>
      <button class="btn primary" disabled={busy} onclick={() => act(() => call("recap_now"))}>{busy ? i18n.t.routines.giving : i18n.t.routines.giveNow}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .activity {
    display: flex;
    flex-direction: column;
    min-height: 0;
    width: 52px;
    border-left: 1px solid var(--rim);
    background: var(--ink);
    transition: width 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .activity.open {
    width: 290px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    /* Under the window bar's icons. */
    padding: 52px 10px 6px 18px;
  }
  .activity:not(.open) header {
    padding-left: 10px;
  }
  h2 {
    flex: 1;
    font: 600 15px/1 var(--display);
    white-space: nowrap;
  }
  .fold {
    display: grid;
    place-items: center;
    width: 32px;
    height: 28px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--haze);
    cursor: pointer;
  }
  .fold:hover {
    background: var(--ink-3);
    color: var(--pearl);
  }
  svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .scroll {
    overflow-y: auto;
    padding: 0 10px 20px;
  }
  section {
    margin-top: 10px;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
  }
  .n {
    padding: 2px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--think) 25%, transparent);
    color: var(--pearl);
    font-size: 10.5px;
    letter-spacing: 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 1px;
  }
  li button {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    padding: 8px;
    border: 0;
    border-radius: 10px;
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  li button:hover {
    background: var(--ink-2);
  }
  li .dot {
    margin-top: 6px;
  }
  .words {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .words b,
  .words span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .words b {
    font-weight: 550;
    font-size: 13.5px;
  }
  .words span {
    color: var(--dusk);
    font-size: 12px;
  }
  .none {
    padding: 2px 8px 4px;
    color: var(--dusk);
    font-size: 12.5px;
  }
  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 10px 18px;
    margin: 0;
  }
  dt {
    color: var(--dusk);
    font-size: 12.5px;
    padding-top: 1px;
  }
  dd {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-wrap: anywhere;
  }
  dd.long {
    display: block;
    white-space: pre-wrap;
    user-select: text;
    line-height: 1.5;
  }
</style>
