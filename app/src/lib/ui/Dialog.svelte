<!--
  A dialog in the middle of the window, over a dimmed page: closed by its
  button, Escape, or a click outside it.
-->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { Snippet } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { i18n } from "../i18n/index.svelte";

  let { title, onclose, children, actions }: { title: string; onclose: () => void; children: Snippet; actions?: Snippet } = $props();

  let box: HTMLElement | undefined = $state();
  $effect(() => box?.focus());
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" transition:fade={{ duration: 140 }} onclick={(e) => e.target === e.currentTarget && onclose()} role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" bind:this={box} transition:scale={{ start: 0.96, duration: 180 }}>
    <header>
      <h2>{title}</h2>
      <button class="close" onclick={onclose} aria-label={i18n.t.home.close} title={i18n.t.home.close}>
        <Icon name="x" size={16} />
      </button>
    </header>
    <div class="body">{@render children()}</div>
    {#if actions}<footer>{@render actions()}</footer>{/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 40px;
    background: var(--scrim);
    backdrop-filter: blur(3px);
  }
  .dialog {
    width: min(560px, 100%);
    max-height: min(640px, 100%);
    display: flex;
    flex-direction: column;
    border-radius: var(--r-lg);
    border: 1px solid var(--rim-2);
    background: var(--ink);
    box-shadow: var(--shadow);
    outline: none;
  }
  header {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 20px 20px 4px 24px;
  }
  h2 {
    flex: 1;
    font: 600 19px/1.3 var(--display);
  }
  .close {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--dusk);
    cursor: pointer;
  }
  .close:hover {
    background: var(--ink-3);
    color: var(--pearl);
  }
  .body {
    overflow-y: auto;
    padding: 12px 24px 20px;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 14px 20px;
    border-top: 1px solid var(--rim);
  }
</style>
