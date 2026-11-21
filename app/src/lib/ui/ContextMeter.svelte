<!--
  How full the conversation's context is: a small ring with the share of the
  model's window (or the tokens, when the window isn't known), and on a
  click the split: Yoru's instructions, his notes, the tools, the
  conversation and the tools' results, each with its tokens.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import type { ContextUse } from "../types";

  let { use }: { use: ContextUse | null } = $props();
  const t = $derived(i18n.t.home.context);

  const COLORS: Record<string, string> = {
    instructions: "var(--pearl-mid)",
    memory: "var(--done)",
    tools: "var(--wait)",
    conversation: "var(--think)",
    results: "#a970ff",
  };

  const share = $derived(use?.window ? Math.min(1, use.tokens / use.window) : null);
  const short = (n: number) => (n >= 1e6 ? `${+(n / 1e6).toFixed(n % 1e6 ? 1 : 0)}M` : n >= 1000 ? `${+(n / 1000).toFixed(n >= 1e5 ? 0 : 1)}k` : `${n}`);
  const percent = (n: number) => (n > 0 && n < 0.01 ? "<1%" : `${Math.round(n * 100)}%`);
  /** Each part's share of what is used: with a 1M window every part would read "<1%". */
  const whole = $derived(Math.max(use?.tokens ?? 1, 1));

  let open = $state(false);
  let root: HTMLElement | undefined = $state();
  function outside(e: PointerEvent) {
    if (open && !root?.contains(e.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={(e) => open && e.key === "Escape" && (open = false)} />

<div class="meter-box" bind:this={root}>
  <button class="icon-btn ring" class:open aria-label={t.label} title={t.label} aria-expanded={open} onclick={() => (open = !open)}>
    <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
      <circle cx="10" cy="10" r="7.5" class="track" />
      <circle cx="10" cy="10" r="7.5" class="fill" class:full={(share ?? 0) > 0.8} pathLength="100" stroke-dasharray="{(share ?? 0) * 100} 100" transform="rotate(-90 10 10)" />
    </svg>
    {#if use}<span class="num">{share !== null ? percent(share) : short(use.tokens)}</span>{/if}
  </button>

  {#if open}
    <div class="pop" role="dialog" aria-label={t.title}>
      <div class="top">
        <b>{t.title}</b>
        {#if use}<span class="faint">{use.model}</span>{/if}
      </div>
      {#if use}
        <p class="total">
          {use.window ? t.of(use.tokens.toLocaleString(i18n.lang), use.window.toLocaleString(i18n.lang)) : t.tokens(use.tokens.toLocaleString(i18n.lang))}
          {#if share !== null}<span class="faint"> · {percent(share)}</span>{/if}
        </p>
        <div class="bar">
          {#each use.parts.filter((p) => p.tokens > 0) as p (p.kind)}
            <span style:width="{(p.tokens / whole) * 100}%" style:background={COLORS[p.kind]} title={t.parts[p.kind]}></span>
          {/each}
        </div>
        <ul>
          {#each use.parts as p (p.kind)}
            <li>
              <span class="dot" style:background={COLORS[p.kind]}></span>
              <span class="name">{t.parts[p.kind]}</span>
              <span class="n">{short(p.tokens)}</span>
              <span class="pc faint">{percent(p.tokens / whole)}</span>
            </li>
          {/each}
          {#if use.window}
            <li>
              <span class="dot free"></span>
              <span class="name">{t.free}</span>
              <span class="n">{short(Math.max(0, use.window - use.tokens))}</span>
              <span class="pc faint">{percent(Math.max(0, 1 - use.tokens / use.window))}</span>
            </li>
          {/if}
        </ul>
        <p class="faint note">{use.exact ? t.split : t.estimate}</p>
      {:else}
        <p class="faint note">{t.none}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .meter-box {
    position: relative;
  }
  .ring {
    display: inline-flex;
    align-items: center;
    width: auto;
    gap: 6px;
    padding: 0 8px;
  }
  .ring.open {
    background: var(--hover);
  }
  .num {
    color: var(--haze);
    font: 500 var(--fs-small)/1 var(--sans);
    font-variant-numeric: tabular-nums;
  }
  .track {
    fill: none;
    stroke: var(--rim-2);
    stroke-width: 2.5;
  }
  .fill {
    fill: none;
    stroke: var(--think);
    stroke-width: 2.5;
    stroke-linecap: round;
    transition: stroke-dasharray 400ms ease;
  }
  .fill.full {
    stroke: var(--wait);
  }
  .pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 50;
    width: 300px;
    padding: 14px 16px;
    border: 1px solid var(--rim-2);
    border-radius: var(--r-lg);
    background: var(--ink);
    box-shadow: var(--shadow), var(--lift);
  }
  .top {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }
  .top .faint {
    overflow: hidden;
    font-size: var(--fs-small);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .total {
    margin: 6px 0 10px;
    font-size: var(--fs-meta);
    font-variant-numeric: tabular-nums;
  }
  .bar {
    display: flex;
    gap: 1px;
    height: 8px;
    overflow: hidden;
    border-radius: 4px;
    background: var(--ink-3);
  }
  .bar span {
    min-width: 2px;
  }
  ul {
    display: grid;
    gap: 6px;
    margin: 12px 0 0;
    padding: 0;
    list-style: none;
    font-size: var(--fs-meta);
  }
  li {
    display: grid;
    grid-template-columns: 10px 1fr auto 40px;
    align-items: center;
    gap: 8px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dot.free {
    background: var(--ink-3);
    box-shadow: inset 0 0 0 1px var(--rim-2);
  }
  .n,
  .pc {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .note {
    margin: 12px 0 0;
    font-size: var(--fs-small);
  }
</style>
