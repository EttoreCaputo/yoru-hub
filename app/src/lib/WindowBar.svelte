<!--
  The window has no system title bar: this strip is where it's dragged from,
  with its own minimize, maximize and close (close only hides it: Yoru stays
  in the tray). Notifications and Settings sit here too, as icons, so they're
  a click away from every page. The window's buttons aren't in the browser
  preview.
-->
<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { inPreview } from "./api";
  import { i18n } from "./i18n/index.svelte";
  import { PAGE_ICONS, SETTINGS_PAGES } from "./icons/pages";
  import { store } from "./store.svelte";

  const t = $derived(i18n.t.window);
  const nav = $derived(i18n.t.nav);
  const win = inPreview ? null : getCurrentWindow();

  const inSettings = $derived(store.page === "settings" || (SETTINGS_PAGES as readonly string[]).includes(store.page));
  // Settings' dot: Yoru can't think yet, a service wants a login, or there's an update.
  const settingsDot = $derived(
    store.info && !store.info.ready
      ? "wrong"
      : store.update || store.connectors.some((c) => c.enabled && (c.state === "needs_login" || c.state === "error"))
        ? "wait"
        : null,
  );
</script>

<div class="bar chrome" data-tauri-drag-region>
  <div class="pages">
    <button class="icon" class:on={store.page === "inbox"} aria-label={nav.inbox} title={nav.inbox} onclick={() => store.go("inbox")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d={PAGE_ICONS.inbox} /></svg>
      {#if store.unread}<span class="count">{store.unread > 99 ? "99+" : store.unread}</span>{/if}
    </button>
    <button class="icon" class:on={inSettings} aria-label={nav.settings} title={nav.settings} onclick={() => store.go("settings")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d={PAGE_ICONS.settings} /></svg>
      {#if settingsDot}<span class="dot {settingsDot}"></span>{/if}
    </button>
  </div>
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
    align-items: flex-start;
  }
  .pages {
    display: flex;
    gap: 4px;
    padding: 6px 4px;
  }
  button.icon {
    position: relative;
    width: 32px;
    height: 28px;
    color: var(--haze);
  }
  button.icon.on {
    background: var(--ink-3);
    color: var(--pearl);
  }
  button.icon svg {
    width: 18px;
    height: 18px;
    stroke-width: 1.6;
    stroke-linejoin: round;
  }
  .count {
    position: absolute;
    top: -2px;
    right: -3px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--pearl);
    color: var(--eye);
    font: 700 10px/16px var(--sans);
    text-align: center;
  }
  button.icon .dot {
    position: absolute;
    top: 3px;
    right: 4px;
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
