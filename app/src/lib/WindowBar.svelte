<!--
  The top of the window, over everything: the macOS title bar is hidden
  behind the page (its own buttons sit at the top left), so this strip is
  where the window is dragged from. Next to the Mac's buttons, the one that
  folds the sidebar away, then the tabs, as in Pocket: the page's (Home,
  Memory…), each conversation or meeting opened, and + for a new
  conversation. On the right, Notifications and Settings, a click
  away from every page, and the one that folds the activity panel.
-->
<script lang="ts">
  import { i18n } from "./i18n/index.svelte";
  import { PAGE_ICONS, SETTINGS_PAGES } from "./icons/pages";
  import { look } from "./look.svelte";
  import { store } from "./store.svelte";
  import { tabs, type Tab } from "./tabs.svelte";
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
  const th = $derived(i18n.t.home);

  /** A tab's name: the conversation's title (or its first words), the meeting's. */
  function tabName(tab: Tab) {
    if (tab.kind === "meeting") {
      const m = store.meetings.find((x) => x.id === tab.id);
      return m?.title ?? i18n.t.meetings.untitled(m ? i18n.date(m.started_at) : "");
    }
    if (tab.id === store.currentChat && store.chatTitle?.text) return store.chatTitle.text;
    const c = store.conversations.find((x) => x.id === tab.id);
    return c?.title || c?.preview || th.untitled;
  }
  // Middle click closes, as in a browser.
  const middle = (e: MouseEvent, at: number) => e.button === 1 && (e.preventDefault(), tabs.close(at));
</script>

<div class="bar chrome" data-tauri-drag-region>
  <button class="icon-btn side" class:on={!look.sidebar} aria-label={sideLabel} title={sideLabel} onclick={() => look.toggleSidebar()}>
    <Icon name="panel-left" />
  </button>

  <div class="tabs" role="tablist" data-tauri-drag-region>
    <button class="tab pagetab" role="tab" class:on={tabs.active < 0} aria-selected={tabs.active < 0} onclick={() => tabs.page()}>
      <Icon name={PAGE_ICONS[store.page]} size={15} />
      <span class="label">{nav[store.page]}</span>
    </button>
    {#each tabs.list as tab, at (tab.kind + tab.id)}
      <div class="tab" role="tab" class:on={tabs.active === at} aria-selected={tabs.active === at} tabindex="-1">
        <button class="open" onclick={() => tabs.select(at)} onauxclick={(e) => middle(e, at)} title={tabName(tab)}>
          <Icon name={tab.kind === "chat" ? "message-circle" : "file-text"} size={14} />
          <span class="label">{tabName(tab)}</span>
        </button>
        <button class="close" aria-label={th.closeTab} title={th.closeTab} onclick={() => tabs.close(at)}><Icon name="x" size={12} stroke={2.25} /></button>
      </div>
    {/each}
    <button class="icon-btn small new" aria-label={th.newChat} title={th.newChat} onclick={() => store.newChat()}><Icon name="plus" size={16} /></button>
  </div>

  <div class="end">
    <button class="icon-btn" class:on={store.page === "inbox"} aria-label={nav.inbox} title={nav.inbox} onclick={() => store.go("inbox")}>
      <Icon name={PAGE_ICONS.inbox} />
      {#if store.unread}<span class="count small">{store.unread > 99 ? "99+" : store.unread}</span>{/if}
    </button>
    <button class="icon-btn" class:on={inSettings} aria-label={nav.settings} title={nav.settings} onclick={() => store.go("settings")}>
      <Icon name={PAGE_ICONS.settings} />
      {#if settingsDot}<span class="dot {settingsDot}"></span>{/if}
    </button>
    <button class="icon-btn" aria-label={panelLabel} title={panelLabel} aria-pressed={look.activity} onclick={() => look.toggleActivity()}>
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
  .tabs {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    margin-left: 10px;
    overflow: hidden;
  }
  /* A tab is a pill on the frame; the one shown is solid, like the page under it. */
  .tab {
    position: relative;
    display: flex;
    align-items: center;
    flex: 0 1 180px;
    min-width: 72px;
    height: 32px;
    border: 1px solid transparent;
    border-radius: 999px;
    color: var(--haze);
    transition:
      background 140ms,
      color 140ms;
  }
  .tab.pagetab {
    flex: 0 0 auto;
    min-width: 0;
    gap: 7px;
    padding: 0 14px 0 12px;
    background: none;
    font: inherit;
    cursor: pointer;
  }
  .tab:hover {
    background: var(--hover);
    color: var(--pearl);
  }
  .tab.on {
    border-color: var(--rim-2);
    background: var(--night);
    color: var(--pearl);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
  }
  .open {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 4px 0 12px;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-meta);
    font-weight: 500;
  }
  .close {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    margin-right: 6px;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--dusk);
    opacity: 0;
    cursor: pointer;
    transition: opacity 120ms;
  }
  .tab:hover .close,
  .tab.on .close {
    opacity: 1;
  }
  .close:hover {
    background: var(--hover);
    color: var(--pearl);
  }
  .new {
    flex-shrink: 0;
    color: var(--haze);
  }
  .end {
    display: flex;
    gap: 2px;
    margin-left: 12px;
    flex-shrink: 0;
  }
</style>
