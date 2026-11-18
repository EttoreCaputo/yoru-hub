<!--
  The top of the window right of the sidebar: the macOS title bar is hidden
  behind the page (its own buttons sit over the sidebar), so this strip is
  where the window is dragged from. Notifications and Settings sit here, as
  icons, so they're a click away from every page.
-->
<script lang="ts">
  import { i18n } from "./i18n/index.svelte";
  import { PAGE_ICONS, SETTINGS_PAGES } from "./icons/pages";
  import { store } from "./store.svelte";

  const nav = $derived(i18n.t.nav);

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
    <button class:on={store.page === "inbox"} aria-label={nav.inbox} title={nav.inbox} onclick={() => store.go("inbox")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d={PAGE_ICONS.inbox} /></svg>
      {#if store.unread}<span class="count">{store.unread > 99 ? "99+" : store.unread}</span>{/if}
    </button>
    <button class:on={inSettings} aria-label={nav.settings} title={nav.settings} onclick={() => store.go("settings")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d={PAGE_ICONS.settings} /></svg>
      {#if settingsDot}<span class="dot {settingsDot}"></span>{/if}
    </button>
  </div>
</div>

<style>
  .bar {
    position: absolute;
    top: 0;
    /* Past the sidebar, whose own header is a drag region. */
    left: 228px;
    right: 10px;
    height: 48px;
    z-index: 5;
    display: flex;
    justify-content: flex-end;
    align-items: flex-start;
  }
  .pages {
    display: flex;
    gap: 4px;
    padding: 10px 8px;
  }
  button {
    position: relative;
    display: grid;
    place-items: center;
    width: 32px;
    height: 28px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--haze);
    cursor: pointer;
  }
  button:hover,
  button.on {
    background: var(--ink-3);
    color: var(--pearl);
  }
  svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
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
  button .dot {
    position: absolute;
    top: 3px;
    right: 4px;
  }
</style>
