<!-- One Untitled UI icon (icons/icons.ts), in the text's colour. `filled`
     draws its filled look (FILLED) where it has one, for what's active. -->
<script lang="ts">
  import { FILLED, ICONS, type IconName } from "../icons/icons";

  let { name, size = 18, stroke = 1.75, filled = false }: { name: IconName; size?: number; stroke?: number; filled?: boolean } = $props();

  const solid = $derived(filled ? FILLED[name] : undefined);
  // The cut-outs are a mask, so they show whatever is behind the icon.
  const mask = `icon-cut-${Math.random().toString(36).slice(2, 9)}`;
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true">
  {#if solid}
    {#if solid.cut}
      <mask id={mask} maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24">
        <rect width="24" height="24" fill="white" stroke="none" />
        <g fill="black" stroke="black">{@html solid.cut}</g>
      </mask>
    {/if}
    <g fill="currentColor" mask={solid.cut ? `url(#${mask})` : undefined}>{@html solid.solid}</g>
    {#if solid.line}{@html solid.line}{/if}
  {:else}
    {@html ICONS[name]}
  {/if}
</svg>

<style>
  .icon {
    flex-shrink: 0;
    display: block;
  }
</style>
