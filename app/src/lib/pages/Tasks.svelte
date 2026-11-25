<!--
  Scheduled ("Pianificate"): the jobs Yoru does on his own, once or on a
  schedule (the morning recap is one), each with how far it may go (read
  and tell, ask before acting, act), when it runs next and what he did last
  time; then the watchers, what he keeps an eye on every so often; then
  what the scheduler did lately.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ScheduleEvent, Task, TaskEntry, Watcher } from "../types";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import TaskEditor from "../ui/TaskEditor.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import WatcherEditor from "../ui/WatcherEditor.svelte";
  import { ago } from "../ui/when";

  const t = $derived(i18n.t.tasks);

  /** The task being edited: a fresh one, or one of the list. */
  let editing = $state<Task | null>(null);
  const fresh = (): Task => ({ id: "", name: "", what: "", repeat: "daily", at: "09:00", days: [], enabled: true, acting: "ask" });

  let running = $state(new Set<string>());
  let errors = $state<Record<string, string>>({});
  async function runNow(id: string) {
    running = new Set([...running, id]);
    const { [id]: _, ...rest } = errors;
    errors = rest;
    try {
      await store.runTask(id);
    } catch (e) {
      errors = { ...errors, [id]: why(e) };
    } finally {
      running = new Set([...running].filter((x) => x !== id));
    }
  }

  async function toggle(e: TaskEntry, enabled: boolean) {
    try {
      await store.putTask({ ...e.task, enabled });
    } catch (err) {
      errors = { ...errors, [e.task.id]: why(err) };
    }
  }

  // The watchers: the one being edited, the ones running now and why one failed.
  const tw = $derived(i18n.t.watching);
  let watching = $state<Watcher | null>(null);
  const freshWatcher = (): Watcher => ({ id: "", name: "", kind: "tool", tool: "", arguments: {}, every_min: 30, enabled: true, instructions: "", max_urgency: "high" });
  let checking = $state(new Set<string>());
  let checkErrors = $state<Record<string, string>>({});
  async function check(id: string) {
    checking = new Set([...checking, id]);
    try {
      await call("run_watcher", { id });
    } catch (e) {
      checkErrors = { ...checkErrors, [id]: why(e) };
    } finally {
      checking = new Set([...checking].filter((x) => x !== id));
      await store.refreshWatchers();
    }
  }
  const watcherName = (id: string) => store.watchers.find((e) => e.watcher.id === id)?.watcher.name ?? id;
  const taskName = (id: string) => store.tasks.find((e) => e.task.id === id)?.task.name ?? id;
  const logLine = (e: ScheduleEvent) => (e.kind === "task" ? tw.logTask(taskName(e.id), !e.error) : tw.logWatched(watcherName(e.id), e.told));

  const when = (at: number) => `${i18n.date(at, { weekday: "short", day: "numeric", month: "short" })} ${i18n.time(at)}`;
</script>

<div class="page">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    {#if !editing}<div class="actions"><button class="btn primary" onclick={() => (editing = fresh())}>{t.add}</button></div>{/if}
  </div>

  {#if editing}
    <TaskEditor task={editing} onclose={() => (editing = null)} />
  {/if}

  {#if store.tasks.length}
    <ul class="list">
      {#each store.tasks as e (e.task.id)}
        {@const task = e.task}
        <li class:off={!task.enabled}>
          <div class="item-head">
            <span class="dot {e.running ? 'think' : e.state.last_error ? 'wrong' : task.enabled ? 'done' : ''}"></span>
            <h3>{task.name}</h3>
            <span class="tag">{t.schedule(task)}</span>
            <span class="tag" class:acts={task.acting === "free"}>{t.actingTag[task.acting]}</span>
            <span class="spacer"></span>
            <Toggle checked={task.enabled} label={t.enabled} onchange={(on) => toggle(e, on)} />
          </div>
          <p class="item-text">{task.what}</p>
          <p class="item-meta">
            {e.next ? t.next(when(e.next)) : task.repeat === "once" && e.state.runs ? t.doneOnce : t.off}
            · {e.state.last_run ? t.lastRun(ago(e.state.last_run, store.now)) : t.never}
            {#if e.state.missed}· {t.missed(e.state.missed)}{/if}
          </p>
          {#if e.state.last_result && !e.state.last_error}<p class="item-meta dim">{e.state.last_result}</p>{/if}
          {#if errors[task.id] ?? e.state.last_error}<p class="error-text">{errors[task.id] ?? e.state.last_error}</p>{/if}
          <div class="item-actions">
            <button class="btn small" disabled={running.has(task.id) || e.running || !store.info?.ready} onclick={() => runNow(task.id)}>{running.has(task.id) || e.running ? t.running : t.runNow}</button>
            <button class="btn quiet small" onclick={() => (editing = structuredClone($state.snapshot(task)) as Task)}>{t.edit}</button>
            <ConfirmButton small label={t.remove} question={t.removeConfirm(task.name)} sure={t.removeSure} onconfirm={() => store.removeTask(task.id)} />
          </div>
        </li>
      {/each}
    </ul>
  {:else if !editing}
    <div class="panel empty"><b>{t.none}</b><p>{t.noneHint}</p></div>
  {/if}

  <div class="section-head">
    <div>
      <h2 class="section-title">{tw.watchers}</h2>
      <p>{tw.watchersHint}</p>
    </div>
    {#if !watching}<button class="btn primary" onclick={() => (watching = freshWatcher())}>{tw.addWatcher}</button>{/if}
  </div>

  {#if watching}
    <WatcherEditor watcher={watching} onclose={() => (watching = null)} />
  {/if}

  {#if store.watchers.length}
    <ul class="list">
      {#each store.watchers as { watcher: w, state } (w.id)}
        <li class:off={!w.enabled}>
          <div class="item-head">
            <span class="dot {state.last_error ? 'wrong' : w.enabled ? 'done' : ''}"></span>
            <h3>{w.name}</h3>
            <span class="tag" class:tool={w.kind === "tool"}>{w.kind === "tool" ? w.tool : i18n.t.watcher.task}</span>
            <span class="spacer"></span>
            <Toggle checked={w.enabled} label={i18n.t.watcher.enabled} onchange={(on) => store.putWatcher({ ...w, enabled: on })} />
          </div>
          {#if w.kind === "task"}<p class="item-text">{w.task}</p>{/if}
          <p class="item-meta">
            {tw.every(w.every_min)} · {state.last_run ? tw.lastRun(ago(state.last_run, store.now)) : tw.never} · {tw.told(state.told)}
          </p>
          {#if state.last_error || checkErrors[w.id]}<p class="error-text">{checkErrors[w.id] ?? state.last_error}</p>{/if}
          <div class="item-actions">
            <button class="btn small" disabled={checking.has(w.id) || !store.info?.ready} onclick={() => check(w.id)}>{checking.has(w.id) ? tw.running : tw.runNow}</button>
            <button class="btn quiet small" onclick={() => (watching = structuredClone($state.snapshot(w)) as Watcher)}>{tw.edit}</button>
            <ConfirmButton small label={tw.remove} question={tw.removeConfirm(w.name)} sure={tw.removeSure} onconfirm={() => store.removeWatcher(w.id)} />
          </div>
        </li>
      {/each}
    </ul>
  {:else if !watching}
    <div class="panel empty"><b>{tw.noWatchers}</b><p>{tw.noWatchersHint}</p></div>
  {/if}

  {#if store.schedule.length}
    <h2 class="section-title">{tw.log}</h2>
    <ul class="log">
      {#each store.schedule as e, i (i)}
        <li>
          <span class="dot {e.error ? 'wrong' : 'done'}"></span>
          <span>{logLine(e)}</span>
          {#if e.error}<span class="error-text">{e.error}</span>{/if}
          <time class="faint">{ago(e.at, store.now)}</time>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .section-head {
    margin-top: 28px;
  }
  .item-head .tag {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .item-head .tag.tool {
    font-family: var(--mono);
    font-weight: 400;
  }
  .log {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: var(--fs-body);
  }
  .log li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 0;
  }
  .log time {
    margin-left: auto;
    font-size: var(--fs-meta);
    white-space: nowrap;
  }
</style>
