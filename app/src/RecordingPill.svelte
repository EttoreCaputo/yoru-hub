<!--
  The recording pill (docs/meetings.md, Recording): a small window under the
  menu bar while a meeting records, with how long it has been going, the
  sound it hears and Stop. The app opens it when a recording starts and
  closes it when the recording is done (src-tauri/src/rec_pill.rs).
-->
<script lang="ts">
  import { call, on } from "./lib/api";
  import { i18n } from "./lib/i18n/index.svelte";

  i18n.start();
  const t = $derived(i18n.t.meetings);

  type Meeting = { started_at: number };
  type MeetingEvent = { kind: string; meeting?: Meeting; levels?: Record<string, number> };

  let startedAt = $state<number | null>(null);
  let now = $state(Date.now());
  let level = $state(0);
  let stopping = $state(false);

  void call<Meeting | null>("meeting_active").then((m) => {
    if (m) startedAt = m.started_at;
  });
  on<MeetingEvent>("meeting", (e) => {
    if (e.kind === "started" && e.meeting) startedAt = e.meeting.started_at;
    // The loudest track: either side talking moves the bars.
    if (e.kind === "level" && e.levels) level = Math.max(0, ...Object.values(e.levels));
  });
  $effect(() => {
    const tick = setInterval(() => (now = Date.now()), 250);
    return () => clearInterval(tick);
  });

  const elapsed = $derived(startedAt === null ? 0 : Math.max(0, Math.floor((now - startedAt) / 1000)));
  const clock = $derived.by(() => {
    const h = Math.floor(elapsed / 3600);
    const m = Math.floor((elapsed % 3600) / 60);
    const s = String(elapsed % 60).padStart(2, "0");
    return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
  });
  // Five bars, the middle ones tallest; silence leaves them short, not flat.
  const SHAPE = [0.45, 0.75, 1, 0.75, 0.45];
  const loud = $derived(Math.min(1, Math.sqrt(level) * 1.4));

  function stop() {
    if (stopping) return;
    stopping = true;
    void call("meeting_stop").catch(() => (stopping = false));
  }
</script>

<div class="pill" role="status" aria-label={t.pillRecording}>
  <span class="bars" aria-hidden="true">
    {#each SHAPE as k, i (i)}
      <i style:height="{4 + 14 * k * Math.max(0.2, loud)}px"></i>
    {/each}
  </span>
  <span class="time">{clock}</span>
  <button class="stop" onclick={stop} disabled={stopping} title={t.pillStop} aria-label={t.pillStop}>
    <span></span>
  </button>
</div>

<style>
  :global(html.pill),
  :global(html.pill body) {
    background: transparent;
    overflow: hidden;
    margin: 0;
  }

  .pill {
    position: absolute;
    top: 6px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 14px;
    height: 48px;
    padding: 0 6px 0 18px;
    border-radius: 999px;
    background: rgba(30, 30, 32, 0.94);
    border: 1px solid rgba(255, 255, 255, 0.1);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.35);
    color: #fff;
    user-select: none;
    -webkit-user-select: none;
    cursor: default;
  }

  .bars {
    display: flex;
    align-items: center;
    gap: 3px;
    height: 20px;
  }

  .bars i {
    width: 3px;
    border-radius: 2px;
    background: #4ade80;
    transition: height 0.2s ease-out;
  }

  .time {
    font: 600 22px/1 -apple-system, "SF Pro Display", system-ui, sans-serif;
    font-variant-numeric: tabular-nums;
    min-width: 52px;
    text-align: center;
  }

  .stop {
    width: 38px;
    height: 38px;
    border-radius: 50%;
    border: 0;
    padding: 0;
    display: grid;
    place-items: center;
    background: rgba(255, 255, 255, 0.1);
    cursor: pointer;
  }

  .stop:hover {
    background: rgba(255, 255, 255, 0.18);
  }

  .stop:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .stop span {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    background: #ef4444;
  }
</style>
