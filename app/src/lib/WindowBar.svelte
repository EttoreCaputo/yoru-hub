<!--
  The top of the window, over everything: the macOS title bar is hidden
  behind the page (its own buttons sit at the top left), so this strip is
  where the window is dragged from. Next to the Mac's buttons, the one that
  folds the sidebar away; on the right, Notifications and Settings, a click
  away from every page, and the one that folds the activity panel.
-->
<script lang="ts">
  import { i18n } from "./i18n/index.svelte";
  import { PAGE_ICONS, SETTINGS_PAGES } from "./icons/pages";
  import { look } from "./look.svelte";
  import { store } from "./store.svelte";
  import Icon from "./ui/Icon.svelte";

  const nav = $derived(i18n.t.nav);
  const tw = $derived(i18n.t.window);

  const inSettings = $derived(store.page === "settings" || (SETTINGS_PAGES as readonly string[]).includes(store.page));
  // Settings' dot: Yoru can't think yet, a service wants a login, or there's an update.
  const settingsDot = $derived(
    store.info && !store.info.ready
      ? "wrong"
      : store.update || store.connectors.some((c) => c.enabled && (c.state === "needs_login" || c.state === "error"))
        ? "wait"
        : null,
  );
  const sideLabel = $derived(look.sidebar ? tw.hideSidebar : tw.showSidebar);
  const panelLabel = $derived(look.activity ? i18n.t.home.hidePanel : i18n.t.home.showPanel);
</script>

<div class="bar chrome" data-tauri-drag-region>
  <button class="side" class:on={!look.sidebar} aria-label={sideLabel} title={sideLabel} onclick={() => look.toggleSidebar()}>
    <Icon name="panel-left" />
  </button>

  <div class="end">
    <button class:on={store.page === "inbox"} aria-label={nav.inbox} title={nav.inbox} onclick={() => store.go("inbox")}>
      <Icon name={PAGE_ICONS.inbox} />
      {#if store.unread}<span class="count">{store.unread > 99 ? "99+" : store.unread}</span>{/if}
    </button>
    <button class:on={inSettings} aria-label={nav.settings} title={nav.settings} onclick={() => store.go("settings")}>
      <Icon name={PAGE_ICONS.settings} />
      {#if settingsDot}<span class="dot {settingsDot}"></span>{/if}
    </button>
    <button aria-label={panelLabel} title={panelLabel} aria-pressed={look.activity} onclick={() => look.toggleActivity()}>
      <Icon name="panel-right" />
      {#if !look.activity && store.working}<span class="dot think"></span>{/if}
    </button>
  </div>
</div>

<style>
  .bar {
    grid-column: 1 / -1;
    grid-row: 1;
    display: flex;
    align-items: center;
    /* Past macOS's own buttons (tauri.conf.json's trafficLightPosition). */
    padding: 2px 14px 0 82px;
    z-index: 5;
  }
  .end {
    display: flex;
    gap: 2px;
    margin-left: auto;
  }
  button {
    position: relative;
    display: grid;
    place-items: center;
    width: 32px;
    height: 30px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--haze);
    cursor: pointer;
    transition:
      background 140ms,
      color 140ms;
  }
  button:hover,
  button.on {
    background: var(--rim);
    color: var(--pearl);
  }
  .count {
    position: absolute;
    top: -1px;
    right: -2px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--pearl);
    color: var(--eye);
    font: 700 var(--fs-small)/16px var(--sans);
    text-align: center;
  }
  button .dot {
    position: absolute;
    top: 4px;
    right: 5px;
  }
</style>
