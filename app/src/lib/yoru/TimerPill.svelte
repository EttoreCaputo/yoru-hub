<!--
  The timer that matters now, as a ring that empties, or a quick start when
  none runs. The board says it out loud when it ends; this is the glance.
-->
<script lang="ts">
  import Pill from "./Pill.svelte";
  import { monitor, type TimerKind } from "../monitor.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { settingsNav } from "../settings/nav.svelte";

  const t = $derived(i18n.t.yoru.timer);
  const clock = $derived(i18n.t.yoru.clock);
  const timer = $derived(monitor.last?.timer);
  const COLORS: Record<TimerKind, string> = { timer: "#f8a639", work: "#ff6347", break: "#40e080", reminder: "#3ae7ed" };
  const accent = $derived(timer ? COLORS[timer.kind] : "#f8a639");
  // Between snapshots a running timer keeps counting down.
  const left = $derived(timer ? Math.max(0, timer.state === "run" ? timer.left_s - Math.max(0, monitor.now - monitor.sentAt) / 1000 : timer.left_s) : 0);
  const frac = $derived(timer && timer.total_s > 0 ? left / timer.total_s : 0);
  const RING = 2 * Math.PI * 26;

  const tool = (name: string, args: Record<string, unknown>) => void monitor.timerTool(name, args).catch(() => {});
</script>

<Pill
  title={t.title}
  {accent}
  chip={timer ? t.kinds[timer.kind] : null}
  chipLive={timer?.state === "run" || timer?.state === "ring"}
  action={{ label: t.all, onclick: () => settingsNav.show("timers") }}
>
  {#if timer}
    <div class="now" class:ring={timer.state === "ring"}>
      <svg viewBox="0 0 64 64" aria-hidden="true">
        <circle cx="32" cy="32" r="26" class="track" />
        <circle cx="32" cy="32" r="26" class="left" stroke-dasharray="{RING * frac} {RING}" transform="rotate(-90 32 32)" />
      </svg>
      <div class="words">
        <span class="left-s" class:paused={timer.state === "pause"}>{clock(Math.ceil(left))}</span>
        <span class="label">
          {timer.label ?? t.kinds[timer.kind]}
          {#if timer.round && timer.rounds} · {t.round(timer.round, timer.rounds)}{/if}
          {#if timer.more} · {t.more(timer.more)}{/if}
        </span>
      </div>
    </div>
    <div class="row">
      {#if timer.state === "run"}<button onclick={() => tool("control_timer", { action: "pause" })}>{t.pause}</button>{/if}
      {#if timer.state === "pause"}<button onclick={() => tool("control_timer", { action: "resume" })}>{t.resume}</button>{/if}
      <button onclick={() => tool("control_timer", { action: "cancel" })}>{timer.state === "ring" ? t.stop : t.cancel}</button>
    </div>
  {:else}
    <p class="empty">{t.none}</p>
    <div class="row">
      {#each [5, 15] as m (m)}
        <button onclick={() => tool("set_timer", { seconds: m * 60 })}>{t.minutes(m)}</button>
      {/each}
      <button class="main" onclick={() => tool("pomodoro", { action: "start", work_minutes: 25, break_minutes: 5, rounds: 4 })}>{t.pomodoro} 25</button>
    </div>
  {/if}
</Pill>

<style>
  .now {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  svg {
    flex: none;
    width: 56px;
    height: 56px;
  }
  .track {
    fill: none;
    stroke: rgba(255, 255, 255, 0.07);
    stroke-width: 6;
  }
  .left {
    fill: none;
    stroke: var(--accent);
    stroke-width: 6;
    stroke-linecap: round;
    transition: stroke-dasharray 400ms linear;
    filter: drop-shadow(0 0 6px color-mix(in srgb, var(--accent) 60%, transparent));
  }
  .ring svg {
    animation: shake 0.6s ease-in-out infinite;
  }
  .words {
    display: grid;
    gap: 6px;
    min-width: 0;
  }
  .left-s {
    font: 300 30px/1 var(--sans);
    letter-spacing: -0.03em;
    font-variant-numeric: tabular-nums;
  }
  .left-s.paused {
    color: var(--dim);
  }
  .label {
    font: 450 12px/1.2 var(--sans);
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .row button {
    padding: 7px 12px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.03);
    color: var(--dim);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
    transition:
      background 160ms,
      color 160ms;
  }
  .row button:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  .row .main {
    background: color-mix(in srgb, #ff6347 16%, transparent);
    border-color: color-mix(in srgb, #ff6347 30%, transparent);
    color: #ffb3a5;
  }
  .empty {
    margin: 0;
    font: 450 12.5px/1.45 var(--sans);
    color: var(--faint);
  }
  @keyframes shake {
    25% {
      transform: rotate(-6deg);
    }
    75% {
      transform: rotate(6deg);
    }
  }
</style>
