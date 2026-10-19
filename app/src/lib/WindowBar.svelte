<!--
  The window has no system title bar: this strip is where it's dragged from,
  with its own minimize, maximize and close (close only hides it: Yoru stays
  in the tray). Nothing to show in the browser preview.
-->
<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { inPreview } from "./api";
  import { i18n } from "./i18n/index.svelte";

  const t = $derived(i18n.t.window);
  const win = inPreview ? null : getCurrentWindow();
</script>

<div class="bar chrome" data-tauri-drag-region>
  {#if win}
    <div class="buttons">
      <button aria-label={t.minimize} title={t.minimize} onclick={() => win.minimize()}><svg viewBox="0 0 12 12"><path d="M2 6h8" /></svg></button>
      <button aria-label={t.maximize} title={t.maximize} onclick={() => win.toggleMaximize()}><svg viewBox="0 0 12 12"><rect x="2.5" y="2.5" width="7" height="7" rx="1.5" /></svg></button>
      <button class="close" aria-label={t.close} title={t.close} onclick={() => win.close()}><svg viewBox="0 0 12 12"><path d="M3 3l6 6M9 3 3 9" /></svg></button>
    </div>
  {/if}
</div>

<style>
  .bar {
    position: absolute;
    top: 0;
    /* Past the sidebar, whose own header is a drag region. */
    left: 228px;
    right: 10px;
    height: 34px;
    z-index: 5;
    display: flex;
    justify-content: flex-end;
  }
  .buttons {
    display: flex;
    gap: 2px;
    padding: 6px 8px;
  }
  button {
    display: grid;
    place-items: center;
    width: 28px;
    height: 22px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--dusk);
    cursor: pointer;
  }
  button:hover {
    background: var(--ink-3);
    color: var(--pearl);
  }
  button.close:hover {
    background: color-mix(in srgb, var(--wrong) 70%, transparent);
  }
  svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
  }
</style>
