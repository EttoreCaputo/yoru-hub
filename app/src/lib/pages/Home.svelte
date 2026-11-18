<!--
  Home, after Pocket's: every conversation with Yoru and every meeting
  recorded, newest first, by day, with a search over their titles; each
  opens in a tab of its own (tabs.svelte.ts). At the bottom the field to ask
  Yoru something, which starts a new conversation in a new tab. With
  nothing yet, Yoru greets you in the middle.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import { tabs } from "../tabs.svelte";
  import type { Meeting } from "../types";
  import Composer from "../ui/Composer.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import ConfirmCard from "../ui/ConfirmCard.svelte";
  import Icon from "../ui/Icon.svelte";
  import YoruBoard from "../YoruBoard.svelte";

  const t = $derived(i18n.t.home);
  const tm = $derived(i18n.t.meetings);
  let query = $state("");
  let error = $state("");

  type Row = { kind: "chat" | "meeting"; id: string; title: string; faint: boolean; note: string; at: number; now: boolean };

  const meetingTitle = (m: Meeting) => m.title ?? tm.untitled(`${i18n.date(m.started_at)} · ${i18n.time(m.started_at)}`);
  const rows = $derived.by<Row[]>(() => {
    const chats: Row[] = store.conversations.map((c) => ({
      kind: "chat",
      id: c.id,
      title: c.title || c.preview || t.untitled,
      faint: !c.title,
      note: c.title ? c.preview : "",
      at: c.updated,
      now: c.id === store.currentChat,
    }));
    const meetings: Row[] = store.meetings.map((m) => ({
      kind: "meeting",
      id: m.id,
      title: meetingTitle(m),
      faint: false,
      note: [m.summarized_at ? t.hasSummary : "", m.transcribed_at ? t.hasTranscript : ""].filter(Boolean).join(" · "),
      at: m.started_at,
      now: m.state === "recording",
    }));
    const q = query.trim().toLowerCase();
    return [...chats, ...meetings].filter((r) => !q || `${r.title} ${r.note}`.toLowerCase().includes(q)).sort((a, b) => b.at - a.at);
  });

  // By day: Today, Yesterday, then the weekday and date.
  const days = $derived.by(() => {
    const groups: { key: string; name: string; date: string; rows: Row[] }[] = [];
    const today = new Date(store.now).toDateString();
    const yesterday = new Date(store.now - 86_400_000).toDateString();
    for (const r of rows) {
      const key = new Date(r.at).toDateString();
      let g = groups.at(-1);
      if (g?.key !== key) {
        const date = i18n.date(r.at, { weekday: "long", day: "numeric", month: "long" });
        const name = key === today ? t.today : key === yesterday ? t.yesterday : "";
        g = { key, name: name || date, date: name ? date : "", rows: [] };
        groups.push(g);
      }
      g.rows.push(r);
    }
    return groups;
  });

  const nothing = $derived(!store.conversations.length && !store.meetings.length);
  const ready = $derived(store.info?.ready ?? true);
  const greeting = $derived.by(() => {
    const h = new Date(store.now).getHours();
    return h < 5 ? t.greeting.night : h < 12 ? t.greeting.morning : h < 18 ? t.greeting.afternoon : h < 23 ? t.greeting.evening : t.greeting.night;
  });
  let reaction = $state<{ name: string; at: number } | null>(null);
  $effect(() => {
    const hello = setTimeout(() => (reaction = { name: "wave", at: Date.now() }), 500);
    return () => clearTimeout(hello);
  });

  const open = (r: Row) => tabs.open({ kind: r.kind, id: r.id });

  async function remove(r: Row) {
    error = "";
    try {
      if (r.kind === "chat") await call("delete_conversation", { id: r.id });
      else {
        await call("delete_meeting", { id: r.id });
        await store.refreshMeetings();
      }
    } catch (e) {
      error = why(e);
    }
  }

  let search: HTMLInputElement | undefined = $state();
  // ⌘K finds, as in Pocket.
  function keys(e: KeyboardEvent) {
    if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      search?.focus();
    }
  }
</script>

<svelte:window onkeydown={keys} />

<section class="home">
  <div class="scroll">
    <div class="page">
      <div class="page-head">
        <div>
          <h1>{t.all}</h1>
          <p class="lede">{t.allLede}</p>
        </div>
        <div class="actions">
          <button class="btn" onclick={() => store.go("meetings")}><span class="rec" aria-hidden="true"></span>{tm.record}</button>
          <button class="btn primary" onclick={() => store.newChat()}><Icon name="plus" size={15} stroke={2.25} />{t.newChat}</button>
        </div>
      </div>

      {#if store.confirmations.length}
        <div class="cards">
          {#each store.confirmations as c (c.id)}<ConfirmCard card={c} />{/each}
        </div>
      {/if}

      {#if nothing}
        <div class="mascot">
          <div class="yoru"><YoruBoard board="live" size={130} {reaction} follow onpoke={() => (reaction = { name: "happy", at: Date.now() })} /></div>
          <p class="eyebrow">{greeting}</p>
          <h2 class="said">{t.noneYet}</h2>
          <p class="dim">{t.noneYetHint}</p>
          {#if ready}
            <div class="suggestions">
              {#each t.suggestions as s (s)}<button class="btn" onclick={() => store.askNew(s)}>{s}</button>{/each}
            </div>
          {:else}
            <button class="btn primary" onclick={() => store.go("settings")}>{t.noKeyAction}</button>
          {/if}
        </div>
      {:else}
        <label class="search">
          <Icon name="search" size={16} />
          <input type="search" bind:this={search} bind:value={query} placeholder={t.search} aria-label={t.search} />
          <kbd>⌘K</kbd>
        </label>
        {#if error}<p class="error-text">{error}</p>{/if}

        {#each days as day (day.key)}
          <h2 class="day">{day.name}{#if day.date}<span>{day.date}</span>{/if}</h2>
          <ul class="rows">
            {#each day.rows as r (r.kind + r.id)}
              <li class:removable={!r.now}>
                <button class="row" onclick={() => open(r)}>
                  <span class="kind {r.kind}" title={r.kind === "chat" ? t.chatKind : t.meetingKind}>
                    <Icon name={r.kind === "chat" ? "message-circle" : "file-text"} size={16} />
                  </span>
                  <span class="what">
                    <span class="name" class:faint={r.faint}>{r.title}</span>
                    {#if r.note}<span class="note">{r.note}</span>{/if}
                  </span>
                  {#if r.now}<span class="tag done">{t.now}</span>{/if}
                  <time class="faint">{i18n.time(r.at)}</time>
                </button>
                {#if !r.now}
                  <span class="remove">
                    <ConfirmButton icon="trash-2" small label={r.kind === "chat" ? t.forget : tm.delete} question={r.kind === "chat" ? t.forgetConfirm : tm.deleteConfirm} sure={r.kind === "chat" ? t.forgetSure : tm.delete} onconfirm={() => remove(r)} />
                  </span>
                {/if}
              </li>
            {/each}
          </ul>
        {:else}
          <p class="faint none">{t.noMatch}</p>
        {/each}
      {/if}
    </div>
  </div>

  <div class="dock">
    <Composer mic conversation="new" />
  </div>
</section>

<style>
  .home {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .page {
    padding-bottom: 24px;
  }
  .rec {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--wrong);
  }
  .cards {
    display: grid;
    gap: 10px;
    margin-bottom: 20px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 8px 0 14px;
    border: 1px solid var(--rim-2);
    border-radius: 999px;
    background: var(--field);
    color: var(--dusk);
    transition: border-color 160ms;
  }
  .search:focus-within {
    border-color: var(--pearl-deep);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: none;
    box-shadow: none;
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  kbd {
    padding: 2px 6px;
    border: 1px solid var(--rim-2);
    border-radius: 6px;
    color: var(--dusk);
    font: 500 var(--fs-small)/1.2 var(--sans);
  }
  .day {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 28px 0 8px;
    font: 600 var(--fs-body)/1.3 var(--sans);
    color: var(--pearl);
  }
  .day span {
    color: var(--dusk);
    font-weight: 400;
    font-size: var(--fs-meta);
  }
  .rows {
    list-style: none;
    margin: 0 -10px;
    padding: 0;
  }
  .rows li {
    position: relative;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-radius: var(--r-md);
    background: none;
    color: var(--pearl);
    text-align: left;
    cursor: pointer;
    transition: background 140ms;
  }
  .row:hover,
  .row:focus-visible {
    background: var(--hover);
  }
  /* A document's icon, as Pocket draws a recording: a small sheet. */
  .kind {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 30px;
    height: 34px;
    border: 1px solid var(--rim-2);
    border-radius: 7px;
    background: var(--night);
    color: var(--dusk);
  }
  .kind.chat {
    border-radius: 50%;
    width: 34px;
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .name,
  .note {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-size: var(--fs-body);
    font-weight: 500;
  }
  .note {
    color: var(--dusk);
    font-size: var(--fs-meta);
  }
  time {
    flex-shrink: 0;
    font-size: var(--fs-meta);
    font-variant-numeric: tabular-nums;
  }
  /* Delete shows over the time, on the row pointed at. */
  .remove {
    position: absolute;
    right: 8px;
    top: 50%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-left: 8px;
    transform: translateY(-50%);
    opacity: 0;
    transition: opacity 140ms;
  }
  .rows li:hover .remove,
  .remove:focus-within {
    opacity: 1;
  }
  .rows li:hover .row {
    background: var(--hover);
  }
  .rows li.removable:hover time {
    visibility: hidden;
  }
  .none {
    margin-top: 28px;
  }
  .mascot {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 24px 0 8px;
    text-align: center;
  }
  .yoru :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .eyebrow {
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .said {
    margin: 4px 0 6px;
    font: 600 var(--fs-head)/1.35 var(--display);
  }
  .suggestions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 18px;
  }
  .mascot > .btn {
    margin-top: 14px;
  }
  .dock {
    width: min(560px, calc(100% - 48px));
    margin: 0 auto;
    padding: 8px 0 20px;
  }
</style>
