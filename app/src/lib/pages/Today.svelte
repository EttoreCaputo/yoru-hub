<!--
  Today: Yoru in its black disc, what it has to say, the field to talk to it;
  then what's unread and what comes next (cards waiting, timers, Claude
  Code at work, the morning recap).
-->
<script lang="ts">
  import { call } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { VoiceState } from "../types";
  import ApprovalCard from "../ui/ApprovalCard.svelte";
  import Composer from "../ui/Composer.svelte";
  import NotificationRow from "../ui/NotificationRow.svelte";
  import { ago, clock } from "../ui/when";
  import YoruBoard from "../YoruBoard.svelte";

  const t = $derived(i18n.t.today);

  const greeting = $derived.by(() => {
    const h = new Date(store.now).getHours();
    return h < 5 ? t.greeting.night : h < 12 ? t.greeting.morning : h < 18 ? t.greeting.afternoon : h < 23 ? t.greeting.evening : t.greeting.night;
  });
  const today = $derived(i18n.date(store.now, { weekday: "long", day: "numeric", month: "long" }));
  const ready = $derived(store.info?.ready ?? true);
  const line = $derived(!ready ? t.noKey : store.unread ? t.unreadLine(store.unread) : t.quietLine);

  // The mirror thinks while an answer is on its way, as the board does.
  const voice = $derived<VoiceState>(store.asking ? "thinking" : store.voice);
  const working = $derived(store.agents.sessions.some((s) => s.state === "work"));
  const badge = $derived(store.agents.approvals.length ? "wait" : working ? "work" : null);
  let reaction = $state<{ name: string; at: number } | null>(null);
  function poke() {
    reaction = { name: "happy", at: Date.now() };
    if (store.boardState === "live") void call("yoru_react", { board: null, name: "happy" }).catch(() => {});
  }

  const unread = $derived(store.inbox.filter((n) => !n.read).slice(0, 5));
  const lastSaid = $derived([...store.chat].reverse().find((l) => l.from === "yoru"));
  const sessions = $derived(store.agents.sessions.filter((s) => s.state === "work" || s.state === "wait"));
  const timers = $derived(store.timers?.timers ?? []);
  const recap = $derived(store.info?.settings.routines.recap ? store.info.settings.routines.recap_at : null);
  const nothingNext = $derived(!store.agents.approvals.length && !sessions.length && !timers.length);

  const timerName = (kind: string) => (kind === "reminder" ? t.reminder : kind === "work" ? t.work : kind === "break" ? t.break : t.timer);
</script>

<div class="page today">
  <section class="hero">
    <div class="disc" class:asleep={store.boardState === "off"}>
      <YoruBoard board={store.boardState === "off" ? "off" : "live"} size={190} {voice} gaze={store.gaze} {badge} {reaction} onpoke={poke} />
    </div>
    <div class="words">
      <p class="eyebrow">{greeting} · <span class="date">{today}</span></p>
      <h1 class="said">{line}</h1>
      {#if !ready}
        <button class="btn primary" onclick={() => store.go("settings")}>{t.noKeyAction}</button>
      {:else}
        <Composer big />
      {/if}
      {#if store.boardState === "off"}<p class="asleep-note faint">{t.asleep}</p>{/if}
    </div>
  </section>

  {#if store.agents.approvals.length}
    <section class="cards">
      {#each store.agents.approvals as a (a.id)}<ApprovalCard approval={a} />{/each}
    </section>
  {/if}

  <section class="cols">
    <div class="col">
      <div class="col-head">
        <h2 class="section-title">{t.unread}</h2>
        <button class="btn quiet small" onclick={() => store.go("inbox")}>{t.allInbox} →</button>
      </div>
      {#if unread.length}
        {#each unread as n (n.id)}<NotificationRow {n} compact />{/each}
      {:else}
        <p class="dim calm">{t.nothingUnread}</p>
      {/if}
      {#if lastSaid}
        <div class="last">
          <h2 class="section-title">{t.lastSaid} · <span class="when">{ago(lastSaid.at, store.now)}</span></h2>
          <button class="quote" onclick={() => store.go("conversation")}>{lastSaid.text}</button>
        </div>
      {/if}
    </div>

    <div class="col">
      <h2 class="section-title">{t.next}</h2>
      <ul class="next">
        {#each timers as tm (tm.id)}
          <li>
            <span class="dot {tm.state === 'ring' ? 'wait' : tm.state === 'run' ? 'think' : ''}"></span>
            <span class="what">{tm.label ?? timerName(tm.kind)}</span>
            <span class="mono dim">{tm.state === "ring" ? t.ringing : tm.state === "pause" ? t.paused : clock(tm.left_s)}</span>
          </li>
        {/each}
        {#each sessions as s (s.id)}
          <li>
            <span class="dot {s.state === 'wait' ? 'wait' : 'think'}"></span>
            <span class="what"><b>{s.project ?? "Claude Code"}</b> {s.task ?? ""}</span>
            <span class="dim small">{t.session[s.state]}</span>
          </li>
        {/each}
        {#if recap}
          <li class="quiet-item">
            <span class="dot"></span>
            <button class="what link" onclick={() => store.go("routines")}>{t.recap(recap)}</button>
          </li>
        {/if}
      </ul>
      {#if nothingNext}<p class="dim calm">{t.nothingNext}</p>{/if}
    </div>
  </section>
</div>

<style>
  .today {
    max-width: 980px;
  }
  .hero {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    gap: 34px;
    padding: 18px 0 30px;
  }
  /* The board's screen: true black, a hair of rim, and Yoru floating in it. */
  .disc {
    width: 240px;
    height: 240px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: radial-gradient(circle at 50% 45%, #050507 55%, #000 72%);
    box-shadow:
      0 0 0 1px var(--rim),
      0 30px 80px -30px rgba(195, 190, 219, 0.18);
    transition: box-shadow 800ms;
  }
  .disc.asleep {
    box-shadow: 0 0 0 1px var(--rim);
  }
  .disc :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .eyebrow {
    color: var(--haze);
    font-size: 13px;
  }
  .date {
    color: var(--dusk);
  }
  .said {
    margin: 8px 0 22px;
    font: 560 34px/1.15 var(--display);
    letter-spacing: -0.015em;
    max-width: 560px;
    text-wrap: balance;
  }
  .asleep-note {
    margin-top: 12px;
    font-size: 12.5px;
  }
  .cards {
    display: grid;
    gap: 10px;
    margin-bottom: 8px;
  }
  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr);
    gap: 40px;
  }
  .col-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .col-head .btn {
    margin-top: 18px;
  }
  .calm {
    padding: 6px 0;
  }
  .last {
    margin-top: 6px;
  }
  .when {
    text-transform: none;
    letter-spacing: 0;
  }
  .quote {
    display: block;
    width: 100%;
    padding: 0 0 0 14px;
    border: 0;
    border-left: 2px solid var(--pearl-deep);
    background: none;
    color: var(--haze);
    font: 400 14px/1.5 var(--sans);
    text-align: left;
    cursor: pointer;
  }
  .quote:hover {
    color: var(--pearl);
  }
  .next {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .next li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 0;
  }
  .next li + li {
    border-top: 1px solid var(--rim);
  }
  .next .what {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .next .what b {
    font-weight: 600;
    margin-right: 4px;
  }
  .small {
    font-size: 12px;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--haze);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .link:hover {
    color: var(--pearl);
  }
  @media (max-width: 900px) {
    .cols {
      grid-template-columns: 1fr;
      gap: 10px;
    }
  }
</style>
