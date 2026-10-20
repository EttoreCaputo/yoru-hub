<!--
  What Yoru does on its own: the morning recap, the quiet hours, and the
  watchers, with what each one did last.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Routines, ScheduleEvent, Watcher } from "../types";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import Toggle from "../ui/Toggle.svelte";
  import WatcherEditor from "../ui/WatcherEditor.svelte";
  import { ago } from "../ui/when";

  const t = $derived(i18n.t.routines);
  const r = $derived(store.info?.settings.routines);

  const set = (change: Partial<Routines>) => store.saveSettings((s) => Object.assign(s.routines, change));

  let recapState = $state<"idle" | "busy" | "done">("idle");
  let recapError = $state("");
  async function recapNow() {
    recapState = "busy";
    recapError = "";
    try {
      await call("recap_now");
      recapState = "done";
    } catch (e) {
      recapError = why(e);
      recapState = "idle";
    }
  }

  /** The watcher being edited: a fresh one, or one of the list. */
  let editing = $state<Watcher | null>(null);
  const fresh = (): Watcher => ({ id: "", name: "", kind: "tool", tool: "", arguments: {}, every_min: 30, enabled: true, instructions: "", max_urgency: "high" });

  let running = $state(new Set<string>());
  let runErrors = $state<Record<string, string>>({});
  async function runNow(id: string) {
    running = new Set([...running, id]);
    try {
      await call("run_watcher", { id });
    } catch (e) {
      runErrors = { ...runErrors, [id]: why(e) };
    } finally {
      running = new Set([...running].filter((x) => x !== id));
      await store.refreshWatchers();
    }
  }

  const toggle = (w: Watcher, enabled: boolean) => store.putWatcher({ ...w, enabled });
  const nameOf = (id: string) => store.watchers.find((e) => e.watcher.id === id)?.watcher.name ?? id;
  const logLine = (e: ScheduleEvent) => (e.kind === "recap" ? t.logRecap : t.logWatched(nameOf(e.id), e.told));
</script>

<div class="page">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  {#if r}
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.recap}</b><span>{t.recapHint}</span></div>
        <div class="how"><Toggle checked={r.recap} label={t.recap} onchange={(on) => set({ recap: on })} /></div>
      </div>
      {#if r.recap}
        <div class="setting">
          <div class="what"><b>{t.at}</b></div>
          <div class="how">
            <label class="inline"><span class="dim">{t.weekdays}</span><Toggle checked={r.recap_weekdays_only} label={t.weekdays} onchange={(on) => set({ recap_weekdays_only: on })} /></label>
            <input type="time" value={r.recap_at} onchange={(e) => set({ recap_at: e.currentTarget.value })} aria-label={t.at} />
          </div>
        </div>
        <div class="setting column">
          <div class="what"><b>{t.instructions}</b><span>{t.instructionsHint}</span></div>
          <textarea rows="3" value={r.recap_task} onchange={(e) => set({ recap_task: e.currentTarget.value })}></textarea>
        </div>
      {/if}
      <div class="row foot">
        <button class="btn" disabled={recapState === "busy" || !store.info?.ready} onclick={recapNow}>{recapState === "busy" ? t.giving : t.giveNow}</button>
        {#if recapState === "done"}<span class="dim">{t.given}</span>{/if}
        {#if recapError}<span class="error-text">{recapError}</span>{/if}
      </div>
    </section>

    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.quiet}</b><span>{t.quietHint}</span></div>
        <div class="how">
          {#if r.quiet}
            <span class="dim">{t.from}</span>
            <input type="time" value={r.quiet_from} onchange={(e) => set({ quiet_from: e.currentTarget.value })} aria-label={t.from} />
            <span class="dim">{t.to}</span>
            <input type="time" value={r.quiet_to} onchange={(e) => set({ quiet_to: e.currentTarget.value })} aria-label={t.to} />
          {/if}
          <Toggle checked={r.quiet} label={t.quiet} onchange={(on) => set({ quiet: on })} />
        </div>
      </div>
    </section>
  {/if}

  <div class="watch-head">
    <div>
      <h2 class="section-title">{t.watchers}</h2>
      <p class="dim hint">{t.watchersHint}</p>
    </div>
    {#if !editing}<button class="btn primary" onclick={() => (editing = fresh())}>{t.addWatcher}</button>{/if}
  </div>

  {#if editing}
    <WatcherEditor watcher={editing} onclose={() => (editing = null)} />
  {/if}

  {#if store.watchers.length}
    <ul class="watchers">
      {#each store.watchers as { watcher: w, state } (w.id)}
        <li class="panel" class:off={!w.enabled}>
          <div class="w-head">
            <span class="dot {state.last_error ? 'wrong' : w.enabled ? 'done' : ''}"></span>
            <h3>{w.name}</h3>
            <span class="tag" class:tool={w.kind === "tool"}>{w.kind === "tool" ? w.tool : i18n.t.watcher.task}</span>
            <span class="spacer"></span>
            <Toggle checked={w.enabled} label={i18n.t.watcher.enabled} onchange={(on) => toggle(w, on)} />
          </div>
          {#if w.kind === "task"}<p class="dim task">{w.task}</p>{/if}
          <p class="facts faint">
            {t.every(w.every_min)} · {state.last_run ? t.lastRun(ago(state.last_run, store.now)) : t.never} · {t.told(state.told)}
          </p>
          {#if state.last_error || runErrors[w.id]}<p class="error-text">{runErrors[w.id] ?? state.last_error}</p>{/if}
          <div class="row actions">
            <button class="btn small" disabled={running.has(w.id) || !store.info?.ready} onclick={() => runNow(w.id)}>{running.has(w.id) ? t.running : t.runNow}</button>
            <button class="btn quiet small" onclick={() => (editing = structuredClone($state.snapshot(w)) as Watcher)}>{t.edit}</button>
            <ConfirmButton small label={t.remove} question={t.removeConfirm(w.name)} sure={t.removeSure} onconfirm={() => store.removeWatcher(w.id)} />
          </div>
        </li>
      {/each}
    </ul>
  {:else if !editing}
    <div class="panel empty"><b>{t.noWatchers}</b><p>{t.noWatchersHint}</p></div>
  {/if}

  {#if store.schedule.length}
    <h2 class="section-title">{t.log}</h2>
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
  .setting.column {
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
  }
  .inline {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-right: 14px;
    font-size: 12.5px;
  }
  .foot {
    padding-top: 12px;
    border-top: 1px solid var(--rim);
    font-size: 12.5px;
  }
  .watch-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 14px;
  }
  .watch-head .section-title {
    margin-bottom: 6px;
  }
  .hint {
    font-size: 13px;
  }
  .watchers {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .watchers li.off h3 {
    color: var(--haze);
  }
  .w-head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .w-head h3 {
    font: 600 15px/1.2 var(--sans);
  }
  .w-head .tag {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .w-head .tag.tool {
    font-family: var(--mono);
    font-weight: 400;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .spacer {
    flex: 1;
  }
  .task {
    margin-top: 8px;
    font-size: 13px;
  }
  .facts {
    margin-top: 6px;
    font-size: 12px;
  }
  .actions {
    margin-top: 10px;
    gap: 4px;
  }
  .log {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 13px;
  }
  .log li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 0;
  }
  .log time {
    margin-left: auto;
    font-size: 12px;
    white-space: nowrap;
  }
</style>
