<!--
  Claude Code, as Yoru sees it: whether it works, and how much of the plan is
  left. The limits come from the status line hook-up; without it only the
  tokens are known.
-->
<script lang="ts">
  import Pill from "./Pill.svelte";
  import Meter from "./Meter.svelte";
  import { activity } from "./activity.svelte";
  import { monitor } from "../monitor.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { settingsNav } from "../settings/nav.svelte";

  const t = $derived(i18n.t.yoru.claude);
  const tok = $derived(i18n.t.yoru.tokens);
  const c = $derived(monitor.last?.claude);
  const tone = $derived(c?.state === "work" ? "work" : activity.claudeDone ? "done" : (c?.state ?? "sleep"));
  const COLORS = { work: "#4aa3ff", done: "#40e080", idle: "#ff9f43", sleep: "#8a8fa0" } as const;
  const usage = (pct: number) => (pct >= 90 ? "#ff4d4d" : pct >= 70 ? "#ff9f43" : "#d97757");
</script>

<Pill
  title={t.title}
  accent={COLORS[tone]}
  chip={c ? t[tone] : null}
  chipLive={tone === "work"}
>
  {#if c}
    <div class="model">
      <span class="name">{c.model ?? "Claude"}</span>
      <span class="today">{t.today} <b>{tok(c.today)}</b></span>
    </div>
    {#if c.s_pct !== undefined}
      <Meter label={t.session} note={c.left_min !== undefined ? t.resets(c.left_min) : ""} value={c.s_pct / 100} text="{c.s_pct}%" color={usage(c.s_pct)} />
    {:else}
      <Meter label={t.window} note={c.left_min !== undefined ? t.resets(c.left_min) : ""} value={Math.min(1, c.tok / 2_000_000)} text={tok(c.tok)} color="#d97757" />
    {/if}
    {#if c.w_pct !== undefined}
      <Meter label={t.week} value={c.w_pct / 100} text="{c.w_pct}%" color={usage(c.w_pct)} />
    {:else}
      <button class="link" onclick={() => settingsNav.go({ tab: "claude", id: "connect" })}>{t.limits}</button>
    {/if}
  {:else}
    <p class="empty">{t.none}</p>
    <button class="link" onclick={() => settingsNav.show("claude")}>{t.connect}</button>
  {/if}
</Pill>

<style>
  .model {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .name {
    font: 600 17px/1 var(--sans);
    letter-spacing: -0.01em;
  }
  .today {
    margin-left: auto;
    font: 500 11.5px/1 var(--sans);
    color: var(--faint);
  }
  .today b {
    font: 550 12px/1 var(--mono);
    color: var(--dim);
  }
  .empty {
    margin: 0;
    font: 450 12.5px/1.45 var(--sans);
    color: var(--dim);
  }
  .link {
    align-self: flex-start;
    border: 0;
    padding: 0;
    background: none;
    color: var(--dim);
    font: 500 12px/1 var(--sans);
    text-decoration: underline;
    text-decoration-color: rgba(255, 255, 255, 0.2);
    text-underline-offset: 3px;
    cursor: pointer;
  }
  .link:hover {
    color: var(--text);
  }
</style>
