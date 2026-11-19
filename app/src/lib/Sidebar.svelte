<!--
  The sidebar: with two boards or more the list of them to pick the one
  shown, the pages (Notifications and Settings are icons in the window bar),
  and at the bottom the Pocket recorder (a click opens its dialog) and the
  board, each connected or not and by what, and "do not disturb". Yoru himself
  isn't here: he sits at the bottom right of the pages (YoruBubble.svelte).
-->
<script lang="ts">
  import { i18n } from "./i18n/index.svelte";
  import { PAGE_ICONS } from "./icons/pages";
  import { look } from "./look.svelte";
  import { store, type Page } from "./store.svelte";
  import { tabs } from "./tabs.svelte";
  import Icon from "./ui/Icon.svelte";

  const t = $derived(i18n.t);

  // Notifications and Settings are icons in the window bar; Settings opens
  // Tasks, Routines, Services, Spoken and Animations.
  const MAIN: Page[] = ["home", "memory", "meetings"];

  // The board and the Pocket, alike: a dot, connected or not, and the icon
  // of what it's connected by.
  const yoru = $derived.by(() => {
    const on = store.boardState === "live" || store.boardState === "boot";
    const by = !on ? null : store.port?.startsWith("wifi:") ? "wifi" : store.port ? "usb" : null;
    return { dot: store.boardState === "live" ? "done" : on ? "think" : "", text: on ? t.link.live : t.link.offline, by } as const;
  });
  const quietUntil = $derived(store.info?.quiet_until ?? null);
  const several = $derived(store.boards.length > 1);
  const boardDot = (link: string) => (link === "connected" ? "done" : link === "searching" ? "think" : "wrong");

  const pocket = $derived.by(() => {
    const p = store.pocket;
    if (!p || p.phase === "off") return null;
    const on = p.phase === "connected";
    return { dot: on ? "done" : "", text: on ? t.pocket.connectedShort : t.pocket.away, by: on ? "bluetooth" : null } as const;
  });

  const badge = (p: Page): { n?: number; dot?: string } | null => {
    if (p === "home" && store.confirmations.length) return { dot: "wait" };
    if (p === "meetings" && store.recording) return { dot: "wrong" };
    return null;
  };
</script>

<nav class="side chrome" class:gone={!look.sidebar} aria-label="Yoru" inert={!look.sidebar}>
  <div class="top" data-tauri-drag-region></div>

  {#if several}
    <div class="boards" role="group" aria-label={t.boards.title}>
      <div class="label">{t.boards.count(store.boards.length)}</div>
      {#each store.boards as b (b.target)}
        <button class="nav-row" class:on={b.current} aria-pressed={b.current} title={t.boards.show(store.boardName(b))} onclick={() => store.showBoard(b)}>
          <span class="dot {boardDot(b.link)}"></span>
          <span class="bname">{store.boardName(b)}</span>
          {#if b.primary}<span class="star" title={t.boards.primary}>★</span>{/if}
          <span class="via">{b.port?.startsWith("wifi:") ? t.link.wifi : b.port ? t.link.usb : "…"}</span>
        </button>
      {/each}
    </div>
  {/if}

  <ul>
    {#each MAIN as p (p)}{@render item(p)}{/each}
  </ul>

  <footer>
    {#if pocket}
      <button class="nav-row device" title={t.pocket.openTitle} onclick={() => (store.pocketShown = true)}>
        {@render device("Pocket", pocket)}
      </button>
    {/if}
    <div class="nav-row device yoru">
      {@render device("Yoru", yoru)}
    </div>
    {#if store.preview}<div class="status"><span class="preview" title={t.nav.previewTitle}>{t.nav.preview}</span></div>{/if}
    {#if quietUntil}
      <div class="dnd on">
        <Icon name="moon" size={16} />
        <span>{t.dnd.on(i18n.time(quietUntil))}</span>
        <button class="btn quiet small" onclick={() => store.doNotDisturb(0)}>{t.dnd.end}</button>
      </div>
    {:else}
      <button class="dnd" onclick={() => store.doNotDisturb(60)} title={t.dnd.hour}>
        <Icon name="moon" size={16} />
        <span>{t.dnd.off}</span>
        <span class="faint hour">{t.dnd.hour}</span>
      </button>
    {/if}
  </footer>
</nav>

{#snippet device(name: string, d: { dot: string; text: string; by: "wifi" | "usb" | "bluetooth" | null })}
  <span class="dot {d.dot}"></span>
  <span class="dname">{name}</span>
  <span class="dstate">{d.text}{#if d.by}<Icon name={d.by} size={14} />{/if}</span>
{/snippet}

{#snippet item(p: Page)}
  {@const b = badge(p)}
  <li>
    <button class="nav-row" class:on={store.page === p && !tabs.shown} aria-current={store.page === p && !tabs.shown ? "page" : undefined} onclick={() => store.go(p)}>
      <Icon name={PAGE_ICONS[p]} />
      <span>{t.nav[p]}</span>
      {#if b?.n}<span class="count">{b.n}</span>{:else if b?.dot}<span class="dot {b.dot}"></span>{/if}
    </button>
  </li>
{/snippet}

<style>
  /* On the window's frame, under its bar: no box of its own. Folded, it
     slides under the page (App.svelte narrows its column to nothing). */
  .side {
    grid-column: 1;
    grid-row: 2;
    display: flex;
    flex-direction: column;
    width: 236px;
    min-height: 0;
    padding: 0 10px 12px;
    transition:
      opacity 200ms,
      visibility 200ms;
  }
  .side.gone {
    opacity: 0;
    visibility: hidden;
  }
  .preview {
    padding: 3px 6px;
    border-radius: 6px;
    background: var(--ink-3);
    color: var(--haze);
    font: 600 var(--fs-small)/1 var(--sans);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .top {
    height: 6px;
  }
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 6px;
    margin: 0 10px 10px;
    color: var(--haze);
    font-size: var(--fs-meta);
    white-space: nowrap;
  }
  .boards {
    display: grid;
    gap: 2px;
    margin: -8px 0 14px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--rim);
  }
  .boards .label {
    padding: 0 10px 6px;
    color: var(--haze);
    font: 600 var(--fs-meta)/1 var(--sans);
  }
  /* The sidebar's rows are quiet until pointed at or open. */
  .nav-row {
    color: var(--haze);
  }
  .nav-row:hover,
  .nav-row.on {
    color: var(--pearl);
  }
  .boards .nav-row {
    gap: 9px;
    min-height: 32px;
  }
  .bname {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .star {
    color: var(--wait);
    font-size: var(--fs-small);
  }
  .via {
    color: var(--dusk);
    font-size: var(--fs-small);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }
  li .nav-row {
    gap: 11px;
    min-height: 36px;
  }
  li .nav-row.on {
    font-weight: 600;
  }
  li span:first-of-type {
    flex: 1;
  }
  footer {
    margin-top: auto;
  }
  /* Pocket and Yoru: the dot on the left, the state and its link on the right. */
  .device {
    gap: 9px;
    min-height: 32px;
    font-size: var(--fs-meta);
  }
  .device.yoru {
    margin-bottom: 8px;
    cursor: default;
  }
  .device.yoru:hover {
    color: var(--haze);
  }
  .dname {
    font-weight: 550;
  }
  .dstate {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .dnd {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    min-height: 38px;
    padding: 0 10px;
    border-radius: var(--r-md);
    border: 1px solid var(--rim);
    background: var(--night);
    color: var(--haze);
    font: 500 var(--fs-meta)/1.2 var(--sans);
    cursor: pointer;
    text-align: left;
  }
  .dnd:hover {
    color: var(--pearl);
  }
  .dnd span {
    white-space: nowrap;
  }
  .dnd .hour {
    margin-left: auto;
    font-size: var(--fs-small);
  }
  .dnd.on {
    cursor: default;
    color: var(--pearl);
    background: var(--night);
    padding-right: 4px;
  }
  .dnd.on span {
    flex: 1;
  }
</style>
