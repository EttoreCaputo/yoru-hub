<!--
  The tasks you give Yoru: jobs he does on his own, once or on a schedule,
  each with how far it may go (read and tell, ask before acting, act), when
  it runs next and what he did last time.
-->
<script lang="ts">
  import { why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Task, TaskEntry } from "../types";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import TaskEditor from "../ui/TaskEditor.svelte";
  import Toggle from "../ui/Toggle.svelte";
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
</div>
