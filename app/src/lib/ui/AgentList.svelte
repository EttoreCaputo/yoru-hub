<!--
  The coding agents at work (the `code_*` tools, docs/assistant.md): every
  conversation with Claude Code, Codex or OpenCode written to in the last
  minutes, the user's own or Yoru's, and Yoru's runs that just ended.
  `dark`: white on black, for the notch.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ActiveAgent } from "../types";
  import { ago } from "./when";

  let { limit = 8, dark = false }: { limit?: number; dark?: boolean } = $props();

  const t = $derived(i18n.t.agents);
  const NAMES: Record<ActiveAgent["agent"], string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode" };
  const folder = (path: string) => path.split("/").filter(Boolean).pop() ?? path;
  const shown = $derived(store.agents.slice(0, limit));
</script>

<ul class="agents" class:dark>
  {#each shown as a (`${a.agent}:${a.run ?? a.session}`)}
    <li>
      <span class="dot {a.state === 'working' ? 'think' : a.state === 'done' ? 'done' : a.state === 'needs_you' ? 'wait' : a.state === 'failed' ? 'wrong' : ''}"></span>
      <div class="what">
        <p class="title">{a.title}</p>
        <p class="meta">
          {NAMES[a.agent]} · {folder(a.project)}{#if a.run !== null} · <span class="yoru">{t.byYoru}</span>{/if}
          · {a.state === "working" ? ago(a.updated, store.now) : t.states[a.state]}
        </p>
        {#if a.now && a.state === "working"}<p class="meta step">{a.now}</p>{/if}
      </div>
      {#if a.run !== null && a.state === "working"}
        <button class="stop" title={t.stop} aria-label={t.stop} onclick={() => store.stopAgent(a.run!)}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="2" /></svg>
        </button>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .agents {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 0;
  }
  li + li {
    border-top: 1px solid var(--rim);
  }
  .what {
    flex: 1;
    min-width: 0;
  }
  .title,
  .meta {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--dusk);
    font-size: 12px;
  }
  .yoru {
    color: var(--haze);
  }
  .step {
    font-family: var(--mono);
    font-size: 11px;
  }
  .dot {
    flex: none;
  }
  .dot.think {
    animation: breathe 1.6s ease-in-out infinite;
  }
  @keyframes breathe {
    50% {
      opacity: 0.35;
    }
  }
  .stop {
    flex: none;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: 1px solid var(--rim);
    border-radius: 50%;
    background: none;
    color: var(--haze);
    cursor: pointer;
  }
  .stop:hover {
    color: var(--pearl);
  }
  .stop svg {
    width: 12px;
    height: 12px;
    fill: currentColor;
  }
  .dark li {
    padding: 6px 0;
  }
  .dark li + li {
    border-top-color: rgba(255, 255, 255, 0.08);
  }
  .dark .title {
    color: rgba(255, 255, 255, 0.92);
    font-size: 12.5px;
  }
  .dark .meta,
  .dark .yoru {
    color: rgba(255, 255, 255, 0.5);
    font-size: 11px;
  }
  .dark .stop {
    border-color: rgba(255, 255, 255, 0.18);
    color: rgba(255, 255, 255, 0.7);
  }
</style>
