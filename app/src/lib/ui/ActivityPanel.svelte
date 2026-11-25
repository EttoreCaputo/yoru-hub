<!--
  The window's right side on every page, on the frame next to the page and
  only as tall as what it lists; the window bar's button folds it away
  entirely (App.svelte shows it). It lists the coding agents at work, the
  scheduled tasks Yoru does on his own (the morning recap among them) and
  the watchers, each a line with what it is and how it stands. A line opens
  its detail in a dialog in the middle of the window.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ActiveAgent, TaskEntry, WatchEntry } from "../types";
  import Dialog from "./Dialog.svelte";
  import { ago } from "./when";

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

  const watchers = $derived(store.watchers.filter((w) => w.watcher.enabled));
  const watcherDot = (w: WatchEntry) => (w.state.last_error ? "wrong" : w.state.last_run ? "done" : "");
  const watcherLine = (w: WatchEntry) =>
    `${i18n.t.watching.every(w.watcher.every_min)} · ${w.state.last_run ? t.ran(ago(w.state.last_run, store.now)) : t.notYet}`;

  // The detail open in the dialog, looked up again so it stays current.
  type Detail = { kind: "agent"; key: string } | { kind: "task"; id: string } | { kind: "watcher"; id: string };
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
  function goToScheduled() {
    detail = null;
    store.go("tasks");
  }
</script>

<aside class="activity" aria-label={t.panel}>
  <h2>{t.panel}</h2>
  <div class="scroll">
      <section>
        <h3 class="section-title">{ta.title}{#if store.working}<span class="tag think">{store.working}</span>{/if}</h3>
        {#if store.agents.length}
          <ul>
            {#each store.agents.slice(0, 6) as a (agentKey(a))}
              <li>
                <button class="nav-row" onclick={() => show({ kind: "agent", key: agentKey(a) })}>
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
                <button class="nav-row" onclick={() => show({ kind: "task", id: e.task.id })}>
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
        <h3 class="section-title">{t.watchers}</h3>
        {#if watchers.length}
          <ul>
            {#each watchers.slice(0, 6) as w (w.watcher.id)}
              <li>
                <button class="nav-row" onclick={() => show({ kind: "watcher", id: w.watcher.id })}>
                  <span class="dot {watcherDot(w)}"></span>
                  <span class="words"><b>{w.watcher.name}</b><span>{watcherLine(w)}</span></span>
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="none">{t.noWatchers}</p>
        {/if}
      </section>
  </div>
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
      <button class="btn quiet" onclick={goToScheduled}>{t.open(i18n.t.nav.tasks)}</button>
      <button class="btn primary" disabled={busy || e.running} onclick={() => act(() => store.runTask(e.task.id))}>{e.running ? tt.running : tt.runNow}</button>
    {/snippet}
  </Dialog>
{:else if detail?.kind === "watcher" && watcher}
  {@const w = watcher}
  <Dialog title={w.watcher.name} onclose={() => (detail = null)}>
    <dl>
      <dt>{t.what}</dt><dd class="long">{w.watcher.kind === "task" ? w.watcher.task : w.watcher.tool}</dd>
      {#if w.watcher.instructions}<dt>{t.instructions}</dt><dd class="long">{w.watcher.instructions}</dd>{/if}
      <dt>{t.every}</dt><dd>{i18n.t.watching.every(w.watcher.every_min)}</dd>
      <dt>{t.last}</dt><dd>{w.state.last_run ? `${when(w.state.last_run)} · ${t.told(w.state.told)}` : t.notYet}</dd>
      {#if w.state.last_error}<dt>{t.error}</dt><dd class="long error-text">{w.state.last_error}</dd>{/if}
    </dl>
    {#if error}<p class="error-text">{error}</p>{/if}
    {#snippet actions()}
      <button class="btn quiet" onclick={goToScheduled}>{t.open(i18n.t.nav.tasks)}</button>
      <button class="btn primary" disabled={busy} onclick={() => act(() => call("run_watcher", { id: w.watcher.id }).then(() => store.refreshWatchers()))}>{i18n.t.watching.runNow}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .activity {
    grid-column: 3;
    grid-row: 2;
    align-self: start;
    display: flex;
    flex-direction: column;
    width: 280px;
    max-height: 100%;
    min-height: 0;
    padding: 6px 12px 12px 2px;
  }
  h2 {
    padding: 0 8px 4px;
    font: 600 var(--fs-head)/1.3 var(--display);
  }
  .scroll {
    overflow-y: auto;
    min-height: 0;
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
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 1px;
  }
  li .nav-row {
    align-items: flex-start;
    padding: 8px;
    font: inherit;
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
    font-size: var(--fs-body);
  }
  .words span {
    color: var(--dusk);
    font-size: var(--fs-meta);
  }
  .none {
    padding: 2px 8px 4px;
    color: var(--dusk);
    font-size: var(--fs-meta);
  }
  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 10px 18px;
    margin: 0;
  }
  dt {
    color: var(--dusk);
    font-size: var(--fs-meta);
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
