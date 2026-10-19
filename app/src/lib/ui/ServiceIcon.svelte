<!--
  A service's mark on a small rounded tile, bundled (icons/services.ts); a
  service it doesn't know gets its initial on a moon instead.
-->
<script lang="ts">
  import { markFor, tone } from "../icons/services";

  let { id, name, url, size = 36, off = false }: { id: string; name: string; url?: string; size?: number; off?: boolean } = $props();
  const mark = $derived(markFor(id, url));
  const kind = $derived(mark ? tone(mark.hex) : "mid");
</script>

{#if mark}
  <span class="mark {kind}" class:off style:--brand={mark.hex} style:--size="{size}px" aria-hidden="true">
    <svg viewBox="0 0 24 24"><path d={mark.path} /></svg>
  </span>
{:else}
  <span class="mark letter" class:off style:--size="{size}px" aria-hidden="true">{name.trim().slice(0, 1).toUpperCase() || "?"}</span>
{/if}

<style>
  .mark {
    display: grid;
    place-items: center;
    width: var(--size);
    height: var(--size);
    border-radius: 28%;
    flex-shrink: 0;
    background: var(--brand);
    color: #fff;
    /* A thin rim, so a tile close to the background still has an edge. */
    box-shadow: inset 0 0 0 1px var(--rim-2);
  }
  .mark svg {
    width: 56%;
    height: 56%;
    fill: currentColor;
  }
  .mark.dark {
    background: var(--pearl);
    color: var(--brand);
  }
  .mark.light {
    color: var(--eye);
  }
  .mark.letter {
    border-radius: 50%;
    background: radial-gradient(circle at 34% 30%, #3a3850, #1c1b26 70%);
    color: var(--pearl-mid);
    font: 650 calc(var(--size) * 0.42) / 1 var(--display);
  }
  .mark.off {
    filter: grayscale(1);
    opacity: 0.6;
  }
</style>
