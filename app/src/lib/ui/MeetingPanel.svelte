<!--
  The window's right side on the Meetings page, where the activity panel is
  on the others (App.svelte swaps them). It shows the meeting picked on the
  calendar: the summary of one recorded (its to-dos ticked off here as on
  the To-do page), the recording that runs, the transcript being made, a
  calendar meeting to come (record it as it starts, without being asked) or
  one past that wasn't recorded (bring in its file). With nothing picked
  it's today: what runs now, what's later, and the meetings' open to-dos.
-->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { addDays, page, startOfDay, type Block } from "../meetings-page.svelte";
  import { store } from "../store.svelte";
  import type { Meeting, Summary } from "../types";
  import { tabs } from "../tabs.svelte";
  import Toggle from "./Toggle.svelte";
  import { clock } from "./when";

  const t = $derived(i18n.t.meetings);
  const MIN = 60_000;
  const b = $derived(page.block);
  let summary = $state<Summary | null | undefined>(undefined);
  let error = $state("");
  let busy = $state(false);

  // The summary of the meeting picked, read again once it's written.
  let summaryFor = "";
  $effect(() => {
    const m = b?.meeting;
    const key = m ? `${m.id}:${m.summarized_at ?? 0}` : "";
    if (key === summaryFor) return;
    summaryFor = key;
    summary = undefined;
    error = "";
    if (!m?.summarized_at) return;
    const id = m.id;
    call<Summary | null>("meeting_summary", { id }).then(
      (s) => b?.meeting?.id === id && (summary = s),
      () => (summary = null),
    );
  });

  const name = (x: Block) => x.title || t.untitled(`${i18n.date(x.start)} · ${i18n.time(x.start)}`);
  const span = (x: Block) => `${i18n.date(x.start, { weekday: "long", day: "numeric", month: "long" })} · ${i18n.time(x.start)}–${i18n.time(x.end)}`;
  const elapsed = $derived(store.recording ? (store.now - store.recording.started_at) / 1000 : 0);
  const todosOf = (id: string) => store.todos.filter((x) => x.meeting === id);

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
  const summarize = (m: Meeting) =>
    act(async () => {
      store.summarizing = { ...store.summarizing, [m.id]: 0 };
      await call("meeting_summarize", { id: m.id, style: null }).catch((e) => {
        const { [m.id]: _, ...rest } = store.summarizing;
        store.summarizing = rest;
        throw e;
      });
    });
  const transcribe = (m: Meeting) =>
    act(async () => {
      store.transcribing = { ...store.transcribing, [m.id]: 0 };
      await call("meeting_transcribe", { id: m.id }).catch((e) => {
        const { [m.id]: _, ...rest } = store.transcribing;
        store.transcribing = rest;
        throw e;
      });
    });
  /** A file for a calendar meeting that wasn't recorded: named after it. */
  const bringIn = (x: Block) =>
    act(async () => {
      const m = await call<Meeting | null>("meeting_import", { path: null });
      if (!m) return;
      if (x.title) await call("rename_meeting", { id: m.id, title: x.title });
      await store.refreshMeetings();
      page.picked = { meeting: m.id };
    });
  const recordNow = (x: Block) => (page.dialog = { mode: "record", title: x.title });

  // Today, with nothing picked.
  const today = $derived(startOfDay(store.now));
  const ofToday = $derived(page.all.filter((x) => x.start >= today && x.start < addDays(today, 1)));
  const now = $derived(ofToday.find((x) => x.state === "live") ?? ofToday.find((x) => x.start <= store.now && x.end > store.now));
  const later = $derived(ofToday.filter((x) => x.start > store.now && x !== now));
  const open = $derived(store.todos.filter((x) => !x.done).slice(0, 8));
  const until = (at: number) => {
    const m = Math.round((at - store.now) / MIN);
    return m < 60 ? t.inMinutes(m) : i18n.time(at);
  };
</script>

<aside class="panel-side" aria-label={b ? name(b) : t.todayTitle}>
  {#if !b}
    <h2>{t.todayTitle}</h2>
    <div class="scroll">
      <section>
        <h3 class="section-title">{t.now}</h3>
        {#if now}
          <button class="item" onclick={() => page.pick(now)}>
            <span class="dot {now.state}"></span>
            <span class="words"><b>{name(now)}</b><span>{now.state === "live" ? `${t.stLive} · ${clock(elapsed)}` : `${i18n.time(now.start)}–${i18n.time(now.end)}`}</span></span>
          </button>
        {:else}
          <p class="none">{t.nothingNow}</p>
        {/if}
      </section>
      <section>
        <h3 class="section-title">{t.later}</h3>
        {#each later as x (x.key)}
          <button class="item" onclick={() => page.pick(x)}>
            <span class="dot {x.state}"></span>
            <span class="words"><b>{name(x)}</b><span>{until(x.start)}{x.event && page.auto.includes(x.event.id) ? ` · ${t.autoBadge}` : ""}</span></span>
          </button>
        {:else}
          <p class="none">{t.nothingLater}</p>
        {/each}
      </section>
      <section>
        <h3 class="section-title">{t.openTodos}{#if store.openTodos}<span class="tag">{store.openTodos}</span>{/if}</h3>
        {#each open as todo (todo.meeting + todo.item.text)}
          <label class="todo">
            <input type="checkbox" checked={todo.done} onchange={(e) => store.setTodo(todo, e.currentTarget.checked)} />
            <span class="words"><span class="text">{todo.item.text}</span><span>{todo.title}</span></span>
          </label>
        {:else}
          <p class="none">{t.noTodos}</p>
        {/each}
      </section>
    </div>
  {:else}
    <div class="top">
      <h2>{name(b)}</h2>
      <button class="icon-btn small" onclick={() => page.pick(null)} aria-label={t.close} title={t.close}><Icon name="x" size={16} /></button>
    </div>
    <p class="when">{span(b)}{#if b.event?.call} · {b.event.call}{/if}{#if b.meeting?.origin === "pocket"} · Pocket{/if}</p>
    <div class="scroll">
      {#if b.state === "live"}
        <div class="live">
          <span class="rec"></span><b>{t.stLive}</b><span class="mono time">{clock(elapsed)}</span>
        </div>
        {#each store.recording?.tracks ?? [] as track (track.name)}
          <div class="track">
            <span class="dim">{track.kind === "mic" ? t.you : t.others}</span>
            <span class="meter level" style:--p={Math.sqrt(store.levels[track.name] ?? 0)}><span></span></span>
          </div>
        {/each}
        <button class="btn wide stop" disabled={!!store.recordBusy} onclick={() => store.stopRecording()}><Icon name="square" size={13} />{t.stop}</button>
      {:else if b.state === "work" && b.meeting}
        {@const m = b.meeting}
        {@const p = store.summarizing[m.id] ?? store.transcribing[m.id] ?? 0}
        <p class="state">{m.id in store.summarizing ? t.summarizing(Math.round(p * 100)) : t.transcribing(Math.round(p * 100))}</p>
        <span class="meter level work" style:--p={p}><span></span></span>
      {:else if b.state === "cal" && b.event}
        {@const ev = b.event}
        <div class="setting">
          <div class="what"><b>{t.autoRecord}</b><span>{t.autoRecordHint}</span></div>
          <div class="how"><Toggle checked={page.auto.includes(ev.id)} label={t.autoRecord} onchange={(on) => act(() => page.setAuto(ev, on))} /></div>
        </div>
        <p class="faint small">{t.sourcesNow(store.recordPlayed === "system" ? t.everything : store.recordPlayed === "none" ? t.nothing : store.recordPlayed)}
          <button class="link" onclick={() => (page.dialog = { mode: "sources" })}>{t.change}</button></p>
        <button class="btn wide" disabled={!!store.recording} onclick={() => recordNow(b)}><span class="rec"></span>{t.recordNow}</button>
      {:else if b.state === "miss"}
        <p class="state">{t.missHint}</p>
        <button class="btn wide" disabled={busy} onclick={() => bringIn(b)}><Icon name="upload" size={15} />{t.importFile}</button>
        {#if store.pocket}<button class="btn wide quiet" onclick={() => (store.pocketShown = true)}><Icon name="phone" size={15} />{t.fromPocket}</button>{/if}
      {:else if b.meeting}
        {@const m = b.meeting}
        {#if summary}
          {#if summary.overview}<p class="overview">{summary.overview}</p>{/if}
          {#if summary.decisions.length}
            <h3 class="section-title">{t.decisions}</h3>
            <ul>{#each summary.decisions as d (d)}<li>{d}</li>{/each}</ul>
          {/if}
          {#if todosOf(m.id).length}
            <h3 class="section-title">{t.actions}</h3>
            {#each todosOf(m.id) as todo (todo.item.text)}
              <label class="todo">
                <input type="checkbox" checked={todo.done} onchange={(e) => store.setTodo(todo, e.currentTarget.checked)} />
                <span class="words"><span class="text">{todo.item.text}</span>{#if todo.item.owner}<span>{todo.item.owner}</span>{/if}</span>
              </label>
            {/each}
          {/if}
          {#if summary.questions.length}
            <h3 class="section-title">{t.questions}</h3>
            <ul>{#each summary.questions as q (q)}<li>{q}</li>{/each}</ul>
          {/if}
        {:else if m.summarized_at && summary === undefined}
          <p class="faint">…</p>
        {:else if m.transcribed_at}
          <p class="state">{t.noSummaryYet}</p>
          <button class="btn wide" disabled={busy} onclick={() => summarize(m)}><Icon name="sparkles" size={15} />{t.summarize}</button>
        {:else}
          <p class="state">{t.noTranscriptYet}</p>
          <button class="btn wide" disabled={busy} onclick={() => transcribe(m)}><Icon name="audio-lines" size={15} />{t.transcribe}</button>
          {#if store.transcribeErrors[m.id]}<p class="error-text">{store.transcribeErrors[m.id]}</p>{/if}
        {/if}
        <button class="btn wide quiet" onclick={() => tabs.open({ kind: "meeting", id: m.id })}><Icon name="file-text" size={15} />{t.openTranscript}</button>
      {/if}
      {#if error}<p class="error-text">{error}</p>{/if}
    </div>
  {/if}
</aside>

<style>
  .panel-side {
    grid-column: 3;
    grid-row: 2;
    display: flex;
    flex-direction: column;
    width: 300px;
    max-height: 100%;
    min-height: 0;
    padding: 6px 14px 14px 4px;
    box-sizing: border-box;
  }
  h2 {
    padding: 0 8px 4px;
    font: 600 var(--fs-head)/1.3 var(--display);
  }
  .top {
    display: flex;
    align-items: flex-start;
    gap: 6px;
  }
  .top h2 {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .when {
    margin: 0 8px 10px;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .when::first-letter {
    text-transform: uppercase;
  }
  .scroll {
    overflow-y: auto;
    min-height: 0;
    padding: 0 8px;
  }
  section {
    margin-top: 10px;
  }
  section .section-title,
  section .none {
    padding: 0;
  }
  .section-title {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 14px;
  }
  .item {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    padding: 7px 6px;
    margin: 0 -6px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .item:hover {
    background: var(--hover);
  }
  .dot {
    width: 8px;
    height: 8px;
    margin-top: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--dusk);
  }
  .dot.live {
    background: var(--wrong);
  }
  .dot.work {
    background: var(--wait);
  }
  .dot.done,
  .dot.recorded {
    background: var(--think);
  }
  .dot.miss {
    background: transparent;
    box-shadow: inset 0 0 0 1px var(--dusk);
  }
  .words {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .words b,
  .words > span:not(.text) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .words b {
    font-weight: 550;
  }
  .words > span:not(.text) {
    color: var(--dusk);
    font-size: var(--fs-meta);
  }
  .none {
    color: var(--dusk);
    font-size: var(--fs-meta);
  }
  .todo {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 6px 0;
    cursor: pointer;
  }
  .todo input {
    margin-top: 3px;
    accent-color: var(--think);
  }
  .todo:has(input:checked) .text {
    color: var(--dusk);
    text-decoration: line-through;
  }
  .overview {
    margin: 0 0 4px;
    line-height: 1.5;
  }
  ul {
    margin: 4px 0 0;
    padding-left: 18px;
    display: grid;
    gap: 4px;
    line-height: 1.45;
  }
  .state {
    margin: 4px 0 10px;
    color: var(--haze);
    line-height: 1.45;
  }
  .wide {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 8px;
    width: 100%;
    margin-top: 10px;
  }
  .live {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 4px 0 12px;
  }
  .time {
    margin-left: auto;
    color: var(--haze);
  }
  .rec {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--wrong);
    flex-shrink: 0;
  }
  .live .rec {
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
  .track {
    display: grid;
    grid-template-columns: 70px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
    margin: 6px 0;
    font-size: var(--fs-meta);
  }
  .level {
    height: 6px;
  }
  .level > span {
    transition-duration: 200ms;
    transition-timing-function: linear;
  }
  .level.work {
    display: block;
  }
  .stop {
    margin-top: 16px;
    color: var(--wrong);
  }
  .small {
    font-size: var(--fs-meta);
    margin: 8px 0 0;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--think);
    font: inherit;
    cursor: pointer;
  }
  .setting {
    padding-left: 0;
    padding-right: 0;
  }
</style>
