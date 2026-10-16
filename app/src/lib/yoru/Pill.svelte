<!--
  One of the cards beside Yoru, like coucou's pills: a title, a state chip in
  the pill's own colour, and whatever the pill shows.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    accent,
    chip = null,
    chipLive = false,
    action = null,
    children,
  }: {
    title: string;
    accent: string;
    chip?: string | null;
    chipLive?: boolean;
    action?: { label: string; onclick: () => void } | null;
    children: Snippet;
  } = $props();
</script>

<article class="pill" style:--accent={accent}>
  <header>
    <h3>{title}</h3>
    {#if chip}<span class="chip" class:live={chipLive}><i></i>{chip}</span>{/if}
    {#if action}<button class="more" onclick={action.onclick}>{action.label} ›</button>{/if}
  </header>
  {@render children()}
</article>

<style>
  .pill {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px 16px 15px;
    border-radius: 20px;
    background:
      radial-gradient(120% 140% at 0% 0%, color-mix(in srgb, var(--accent) 8%, transparent), transparent 55%),
      linear-gradient(180deg, rgba(255, 255, 255, 0.045), rgba(255, 255, 255, 0.015));
    border: 1px solid var(--line);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.05),
      0 20px 50px -30px rgba(0, 0, 0, 0.9);
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 20px;
  }
  h3 {
    margin: 0;
    font: 650 11px/1 var(--sans);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: 999px;
    font: 550 11px/1 var(--sans);
    color: color-mix(in srgb, var(--accent) 70%, var(--text));
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .chip i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .chip.live i {
    box-shadow: 0 0 8px var(--accent);
    animation: breathe 1.6s ease-in-out infinite;
  }
  .more {
    margin-left: auto;
    border: 0;
    background: none;
    padding: 2px 0;
    color: var(--faint);
    font: 500 11.5px/1 var(--sans);
    cursor: pointer;
  }
  .more:hover {
    color: var(--text);
  }
  @keyframes breathe {
    50% {
      opacity: 0.35;
    }
  }
</style>
