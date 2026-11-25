<!--
  Meetings (docs/meetings.md): a calendar of the week or the day, with the
  meetings in the Mac's calendar and what was recorded laid on the same
  hours, one block each (a recording made during a calendar meeting is that
  meeting). Picking a block opens it in the panel on the window's right
  (MeetingPanel.svelte): its summary, the recording that runs, or whether
  to record it when it starts. New meeting records now (the dialog keeps
  what the window bar's Record records), brings in an audio or video file
  (or one dropped on the window), or opens the Pocket. Past lists every
  recording. The field at the bottom asks Yoru, about the meeting picked.
-->
<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  import { call, inPreview, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { addDays, page, startOfDay, type Block } from "../meetings-page.svelte";
  import { store } from "../store.svelte";
  import type { Meeting } from "../types";
  import { tabs } from "../tabs.svelte";
  import Composer from "../ui/Composer.svelte";
  import RecordDialog from "../ui/RecordDialog.svelte";

  const t = $derived(i18n.t.meetings);
  const HOUR = 56;
  const MIN = 60_000;
  let error = $state("");
  let importing = $state(false);
  let dropping = $state(false);
  let menu = $state(false);
  let search = $state("");
  let body: HTMLElement | undefined = $state();
  let searchField: HTMLInputElement | undefined = $state();

  $effect(() => {
    void page.readCalendar();
    void page.readSettings();
    const timer = setInterval(() => void page.readCalendar(), 60_000);
    return () => clearInterval(timer);
  });

  // Audio or video files dropped on the window become meetings.
  $effect(() => {
    if (inPreview) return;
    let unlisten: (() => void) | undefined;
    let gone = false;
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent(({ payload }) => {
          if (payload.type === "enter" || payload.type === "over") dropping = true;
          else if (payload.type === "leave") dropping = false;
          else {
            dropping = false;
            for (const path of payload.paths) void bringIn(path);
          }
        }),
      )
      .then((stop) => (gone ? stop() : (unlisten = stop)));
    return () => {
      gone = true;
      unlisten?.();
    };
  });

  /** A file as a new meeting: `path`, or one picked in Finder. */
  async function bringIn(path?: string) {
    menu = false;
    importing = true;
    error = "";
    try {
      const m = await call<Meeting | null>("meeting_import", { path: path ?? null });
      await store.refreshMeetings();
      if (m) page.picked = { meeting: m.id };
    } catch (e) {
      error = why(e);
    } finally {
      importing = false;
    }
  }

  // The days shown: the weekdays, and Saturday and Sunday only when there's
  // something on them or one of them is today.
  const today = $derived(startOfDay(store.now));
  const days = $derived.by(() => {
    const all = Array.from({ length: page.days }, (_, i) => addDays(page.from, i));
    if (page.view === "day") return all;
    return all.filter((d, i) => i < 5 || d === today || page.all.some((b) => b.start >= d && b.start < addDays(d, 1)));
  });

  // The hours: 8 to 19, stretched to what's on the days shown.
  const shown = $derived(page.all.filter((b) => days.some((d) => b.start < addDays(d, 1) && b.end > d)));
  const first = $derived(Math.min(8, ...shown.map((b) => new Date(b.start).getHours())));
  const last = $derived(Math.max(19, ...shown.map((b) => { const e = new Date(b.end); return e.getHours() + (e.getMinutes() ? 1 : 0) || 24; })));
  const hours = $derived(Array.from({ length: last - first + 1 }, (_, i) => first + i));
  const y = (at: number, day: number) => ((at - day) / MIN / 60 - first) * HOUR;

  /** Where each block goes in its day: side by side when they overlap. */
  function lanes(day: number) {
    const end = addDays(day, 1);
    const list = page.all.filter((b) => b.start < end && b.end > day);
    const placed: { b: Block; lane: number; of: number }[] = [];
    let group: typeof placed = [];
    let groupEnd = 0;
    const close = () => {
      const of = Math.max(...group.map((g) => g.lane)) + 1;
      for (const g of group) g.of = of;
    };
    for (const b of list) {
      if (group.length && b.start >= groupEnd) {
        close();
        group = [];
      }
      const free = [0, 1, 2, 3, 4, 5].find((l) => !group.some((g) => g.lane === l && g.b.end > b.start)) ?? 0;
      const p = { b, lane: free, of: 1 };
      group.push(p);
      placed.push(p);
      groupEnd = Math.max(groupEnd, b.end);
    }
    if (group.length) close();
    return placed;
  }

  // The view starts around now, or at the first meeting of the day shown.
  let scrolledFor = "";
  $effect(() => {
    const key = `${page.view}:${page.from}`;
    if (!body || scrolledFor === key) return;
    scrolledFor = key;
    const inView = today >= page.from && today < page.to;
    const at = inView ? store.now : (shown[0]?.start ?? page.from + first * 3_600_000);
    body.scrollTop = Math.max(0, y(at, startOfDay(at)) - HOUR);
  });

  const month = $derived(i18n.date(page.view === "day" ? page.from : addDays(page.from, 3), { month: "long", year: "numeric" }));
  const name = (b: Block) => b.title || t.untitled(`${i18n.date(b.start)} · ${i18n.time(b.start)}`);
  const sub = (b: Block) =>
    b.state === "live"
      ? t.stLive
      : b.state === "work"
        ? t.stWork
        : b.state === "miss"
          ? t.stMiss
          : b.state === "cal"
            ? page.auto.includes(b.event!.id) ? t.autoBadge : b.event!.call || `${i18n.time(b.start)}–${i18n.time(b.end)}`
            : b.meeting?.origin === "pocket"
              ? "Pocket"
              : `${Math.max(1, Math.round((b.meeting?.duration_ms ?? 0) / MIN))} min`;

  function open(b: Block) {
    if (b.meeting && b.state !== "live") tabs.open({ kind: "meeting", id: b.meeting.id });
  }

  // Past: every recording, newest first, kept to the search.
  const past = $derived(
    store.meetings.filter((m) => m.state !== "recording" && (!search.trim() || (m.title ?? "").toLowerCase().includes(search.trim().toLowerCase()))),
  );
  function showPast(focus = false) {
    page.past = true;
    menu = false;
    if (focus) setTimeout(() => searchField?.focus());
  }

  const picked = $derived(page.block);
  const about = $derived(picked?.meeting ? t.aboutPrefix(name(picked), picked.meeting.id) : undefined);
</script>

<svelte:window onclick={() => (menu = false)} />

<div class="meetings">
  {#if dropping}<div class="drop" aria-hidden="true"><span>{t.dropHere}</span></div>{/if}

  {#if page.past}
    <div class="head">
      <button class="icon-btn" onclick={() => (page.past = false)} aria-label={t.backToCalendar} title={t.backToCalendar}><Icon name="chevron-left" size={18} /></button>
      <h1>{t.pastTitle}</h1>
    </div>
    <label class="search">
      <Icon name="search" size={16} />
      <input type="search" bind:this={searchField} bind:value={search} placeholder={t.searchPast} aria-label={t.searchPast} />
    </label>
    <div class="list">
      {#each past as m (m.id)}
        {@const b = page.all.find((x) => x.meeting?.id === m.id)}
        <button class="row" class:on={page.picked?.meeting === m.id} onclick={() => (page.picked = { meeting: m.id })} ondblclick={() => tabs.open({ kind: "meeting", id: m.id })}>
          <span class="when">{i18n.date(m.started_at, { weekday: "short", day: "numeric", month: "short" })}</span>
          <span class="what">
            <b>{b ? name(b) : (m.title ?? t.untitled(i18n.time(m.started_at)))}</b>
            {#if m.origin === "pocket"}<Icon name="phone" size={13} />{/if}
          </span>
          <span class="tag {b?.state ?? 'recorded'}">{b?.state === "work" ? t.stWork : m.summarized_at ? t.stSummarized : m.transcribed_at ? t.stTranscribed : t.stRecorded}</span>
          <span class="dur">{Math.max(1, Math.round(m.duration_ms / MIN))} min</span>
        </button>
      {:else}
        <p class="faint none">{search.trim() ? t.noMatchPast : t.noPast}</p>
      {/each}
    </div>
  {:else}
    <div class="head">
      <h1>{t.title}</h1>
      <span class="grow"></span>
      <button class="icon-btn" onclick={() => showPast(true)} aria-label={t.searchPast} title={t.searchPast}><Icon name="search" size={17} /></button>
      <button class="btn" onclick={() => showPast()}>{t.past}</button>
      <div class="menu-wrap">
        <button class="btn primary" disabled={importing} aria-haspopup="menu" aria-expanded={menu} onclick={(e) => (e.stopPropagation(), (menu = !menu))}>
          {importing ? t.importing : t.newMeeting}<Icon name="chevron-down" size={15} stroke={2} />
        </button>
        {#if menu}
          <div class="menu" role="menu">
            <button role="menuitem" disabled={!!store.recording} onclick={() => ((menu = false), (page.dialog = { mode: "record" }))}><span class="rec"></span>{t.recordNow}</button>
            <button role="menuitem" onclick={() => bringIn()}><Icon name="upload" size={16} />{t.importFile}</button>
            {#if store.pocket}<button role="menuitem" onclick={() => ((menu = false), (store.pocketShown = true))}><Icon name="phone" size={16} />{t.fromPocket}</button>{/if}
            <hr />
            <button role="menuitem" class="quiet" onclick={() => ((menu = false), (page.dialog = { mode: "sources" }))}><Icon name="sliders" size={16} />{t.sources}</button>
          </div>
        {/if}
      </div>
    </div>

    <div class="tools">
      <button class="icon-btn" onclick={() => page.step(-1)} aria-label={t.previous} title={t.previous}><Icon name="chevron-left" size={18} /></button>
      <button class="icon-btn" onclick={() => page.step(1)} aria-label={t.next} title={t.next}><Icon name="chevron-right" size={18} /></button>
      <span class="month">{month}</span>
      <button class="btn small quiet" onclick={() => page.today()}>{t.today}</button>
      <span class="grow"></span>
      <div class="seg" role="radiogroup" aria-label={t.view}>
        <button role="radio" aria-checked={page.view === "day"} class:on={page.view === "day"} onclick={() => page.show("day")}>{t.dayView}</button>
        <button role="radio" aria-checked={page.view === "week"} class:on={page.view === "week"} onclick={() => page.show("week")}>{t.weekView}</button>
      </div>
    </div>

    {#if page.calendarError}
      <p class="note-line">
        {page.calendarError === "denied" ? t.calendarDenied : t.calendarFailed(page.calendarError)}
        {#if page.calendarError === "denied"}
          <button class="btn small" onclick={() => call("open_privacy", { kind: "calendar" }).catch((e) => (error = why(e)))}>{t.openPrivacy}</button>
        {/if}
      </p>
    {/if}

    <div class="cal" style:--n={days.length}>
      <div class="days">
        <span></span>
        {#each days as d (d)}
          <div class="day" class:today={d === today}>
            {i18n.date(d, { weekday: "short" })}
            <b>{new Date(d).getDate()}</b>
          </div>
        {/each}
      </div>
      <div class="body" bind:this={body}>
        <div class="grid" style:height="{(hours.length - 1) * HOUR + 16}px">
          {#each hours as h, i (h)}
            <span class="hour" style:top="{i * HOUR + 8}px">{i ? h : ""}</span>
            <span class="line" style:top="{i * HOUR + 8}px"></span>
          {/each}
          <div class="cols">
            {#each days as d (d)}
              <div class="col">
                {#each lanes(d) as { b, lane, of } (b.key)}
                  {@const top = Math.max(0, y(b.start, d)) + 9}
                  {@const height = Math.max(20, y(Math.min(b.end, addDays(d, 1)), d) - Math.max(0, y(b.start, d)) - 3)}
                  <button
                    class="block {b.state}"
                    class:sel={picked?.key === b.key}
                    style:top="{top}px"
                    style:height="{height}px"
                    style:left="calc({(lane / of) * 100}% + 3px)"
                    style:width="calc({100 / of}% - 6px)"
                    onclick={(e) => (e.stopPropagation(), page.pick(picked?.key === b.key ? null : b))}
                    ondblclick={() => open(b)}
                    title={name(b)}
                  >
                    <span class="name">{#if b.state === "live"}<span class="rec"></span>{/if}{name(b)}</span>
                    {#if height > 34}<small>{sub(b)}</small>{/if}
                  </button>
                {/each}
                {#if d === today}<span class="now" style:top="{y(store.now, d) + 8}px"></span>{/if}
              </div>
            {/each}
          </div>
        </div>
      </div>
    </div>
  {/if}

  {#if error}<p class="error-text">{error}</p>{/if}
  {#if store.recordingError?.code === "no_system_audio"}
    <p class="error-text">
      {t.noSystemAudio}
      <button class="btn small" onclick={() => call("open_audio_privacy").catch((e) => (error = why(e)))}>{t.openPrivacy}</button>
    </p>
  {:else if store.recordingError?.code === "no_microphone_sound"}
    <p class="error-text">{t.noMicSound}</p>
  {:else if store.recordingError}
    <p class="error-text">{store.recordingError.message}</p>
  {/if}

  <div class="dock">
    <Composer mic conversation="new" {about} placeholder={picked?.meeting ? t.askAbout(name(picked)) : page.past ? t.askPast : t.askWeek} />
  </div>
</div>

{#if page.dialog}<RecordDialog />{/if}

<style>
  .meetings {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 26px 32px 0;
    box-sizing: border-box;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
  }
  .grow {
    flex: 1;
  }
  .head .btn :global(svg) {
    margin-left: 4px;
  }
  .menu-wrap {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 20;
    width: 260px;
    padding: 6px;
    border-radius: var(--r);
    background: var(--ink-2);
    box-shadow: 0 0 0 1px var(--rim-2), var(--shadow);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .menu button:hover:not(:disabled) {
    background: var(--hover);
  }
  .menu button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .menu .quiet {
    color: var(--haze);
  }
  .menu hr {
    border: 0;
    border-top: 1px solid var(--rim);
    margin: 4px 6px;
  }
  .rec {
    display: inline-block;
    width: 9px;
    height: 9px;
    margin: 0 3px;
    border-radius: 50%;
    background: var(--wrong);
    flex-shrink: 0;
  }
  .block .rec {
    width: 7px;
    height: 7px;
    margin: 0 5px 1px 0;
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-bottom: 10px;
  }
  .month {
    margin: 0 8px 0 6px;
    font: 600 var(--fs-head)/1 var(--display);
    white-space: nowrap;
  }
  .month::first-letter {
    text-transform: uppercase;
  }
  .note-line {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 0 10px;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .cal {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-radius: var(--r);
    background: var(--ink);
    box-shadow: 0 0 0 1px var(--rim);
    overflow: hidden;
  }
  .days {
    display: grid;
    grid-template-columns: 38px repeat(var(--n), minmax(0, 1fr));
    padding: 10px 0 8px;
    border-bottom: 1px solid var(--rim);
  }
  .day {
    text-align: center;
    color: var(--haze);
    font-size: var(--fs-small);
  }
  .day b {
    display: block;
    width: 30px;
    height: 30px;
    margin: 3px auto 0;
    border-radius: 50%;
    font: 500 15px/30px var(--display);
    color: var(--pearl);
  }
  .day.today b {
    background: var(--think);
    color: #fff;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .grid {
    position: relative;
  }
  .hour {
    position: absolute;
    left: 0;
    width: 30px;
    text-align: right;
    transform: translateY(-7px);
    color: var(--dusk);
    font-size: 11px;
  }
  .line {
    position: absolute;
    left: 38px;
    right: 0;
    border-top: 1px solid var(--rim);
  }
  .cols {
    position: absolute;
    inset: 0 0 0 38px;
    display: grid;
    grid-template-columns: repeat(var(--n), minmax(0, 1fr));
  }
  .col {
    position: relative;
    border-left: 1px solid var(--rim);
  }
  .block {
    position: absolute;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: 4px 7px;
    border: 0;
    border-radius: 7px;
    overflow: hidden;
    font: 500 12px/1.25 var(--sans);
    text-align: left;
    cursor: pointer;
  }
  .block .name,
  .block small {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .block small {
    font-size: 11px;
    font-weight: 400;
    opacity: 0.75;
  }
  .block.done,
  .block.recorded {
    background: color-mix(in srgb, var(--think) 16%, var(--ink));
    color: color-mix(in srgb, var(--think) 55%, var(--pearl));
    box-shadow: inset 3px 0 0 var(--think);
  }
  .block.recorded {
    box-shadow: inset 3px 0 0 color-mix(in srgb, var(--think) 45%, transparent);
  }
  .block.work {
    background: color-mix(in srgb, var(--wait) 14%, var(--ink));
    color: color-mix(in srgb, var(--wait) 60%, var(--pearl));
    box-shadow: inset 3px 0 0 var(--wait);
  }
  .block.live {
    background: color-mix(in srgb, var(--wrong) 18%, var(--ink));
    color: color-mix(in srgb, var(--wrong) 55%, var(--pearl));
    box-shadow: inset 3px 0 0 var(--wrong), 0 0 0 1px color-mix(in srgb, var(--wrong) 45%, transparent);
  }
  .block.cal {
    background: var(--ink-2);
    color: var(--pearl-mid);
    box-shadow: inset 0 0 0 1px var(--rim-2);
  }
  .block.miss {
    background: transparent;
    color: var(--dusk);
    border: 1px dashed var(--rim-2);
  }
  .block.sel {
    outline: 2px solid var(--pearl-mid);
    outline-offset: 1px;
    z-index: 2;
  }
  .now {
    position: absolute;
    left: -1px;
    right: 0;
    z-index: 3;
    border-top: 1.5px solid var(--wrong);
    pointer-events: none;
  }
  .now::before {
    content: "";
    position: absolute;
    left: -4px;
    top: -4.5px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--wrong);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    margin-bottom: 10px;
    border-radius: var(--r);
    background: var(--ink-2);
    box-shadow: 0 0 0 1px var(--rim);
    color: var(--dusk);
  }
  .search input {
    flex: 1;
    padding: 9px 0;
    border: 0;
    background: none;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .row {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr) auto 60px;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 11px 8px;
    border: 0;
    border-top: 1px solid var(--rim);
    background: none;
    color: var(--pearl);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .row:hover,
  .row.on {
    background: var(--hover);
  }
  .row .when,
  .row .dur {
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .row .dur {
    text-align: right;
  }
  .row .what {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--dusk);
  }
  .row .what b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--pearl);
    font-weight: 600;
  }
  .tag.work {
    color: var(--wait);
  }
  .none {
    padding: 20px 8px;
  }
  .dock {
    width: min(560px, calc(100% - 48px));
    margin: 0 auto;
    padding: 12px 0 20px;
  }
  .drop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--night) 70%, transparent);
    border: 2px dashed var(--rim-2);
    pointer-events: none;
  }
  .drop span {
    font: 600 var(--fs-head)/1.2 var(--display);
    color: var(--pearl);
  }
</style>
