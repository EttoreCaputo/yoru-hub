<!--
  The sidebar: with two boards or more the list of them to pick the one
  shown, the pages (Notifications and Settings are icons in the window bar),
  the user's folders (a click shows one on Home, a row dropped on one goes
  in it; docs/app.md),
  and at the bottom the Pocket recorder (a click opens its dialog), iCloud
  sync while it's on (a click opens its Settings page) and the board, each connected or not and by what, and "do not disturb". Yoru himself
  isn't here: he sits at the bottom right of the pages (YoruBubble.svelte).
-->
<script lang="ts">
  import { onWifi } from "./port";
  import { i18n } from "./i18n/index.svelte";
  import { PAGE_ICONS } from "./icons/pages";
  import { look } from "./look.svelte";
  import { store, type Page } from "./store.svelte";
  import { tabs } from "./tabs.svelte";
  import Icon from "./ui/Icon.svelte";
  import { why } from "./api";
  import { ago } from "./ui/when";

  const t = $derived(i18n.t);

  // Notifications and Settings are icons in the window bar; Settings opens
  // Tasks, Routines, Services, Spoken and Animations.
  const MAIN: Page[] = ["home", "memory", "meetings", "todos"];

  // The board and the Pocket, alike: a dot, connected or not, and the icon
  // of what it's connected by.
  const yoru = $derived.by(() => {
    const on = store.boardState === "live" || store.boardState === "boot";
    const by = !on ? null : onWifi(store.port) ? "wifi" : store.port ? "usb" : null;
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

  // iCloud sync, while it's on: a dot, when it last synced, and a cloud.
  const icloud = $derived.by(() => {
    if (!store.icloudOn) return null;
    const c = store.icloud;
    const ti = t.icloud;
    if (!c) return { dot: "think", text: ti.side.starting, by: "cloud" } as const;
    const last = Math.max(c.last_sent, c.last_fetched);
    if (c.errors.length && c.last_error > last) return { dot: "wrong", text: ti.side.error, by: "cloud" } as const;
    if (c.account !== "available") return { dot: "wait", text: ti.side.noAccount, by: "cloud" } as const;
    return { dot: "done", text: last ? ago(last, store.now) : ti.side.waiting, by: "cloud" } as const;
  });
  function openICloud() {
    store.settingsPart = "icloud";
    store.go("settings");
  }

  // Folders: Home showing one of them, a new one being named, the one an
  // item is dragged over.
  const tf = $derived(t.folders);
  let naming = $state(false);
  let name = $state("");
  let folderError = $state("");
  let over = $state<string | null>(null);
  const inFolder = (id: string) => store.folderCount(id);
  const folderOn = (id: string) => store.page === "home" && store.folder === id && !tabs.shown;

  // Enter and the blur that follows both land here: one create at a time.
  let busy = false;
  async function create() {
    if (!naming || busy) return;
    const n = name.trim();
    if (!n) return (naming = false);
    busy = true;
    try {
      await store.changeFolders("create_folder", { name: n });
      naming = false;
      name = "";
      folderError = "";
    } catch (e) {
      folderError = why(e);
    } finally {
      busy = false;
    }
  }

  function nameKeys(e: KeyboardEvent) {
    if (e.key === "Enter") void create();
    else if (e.key === "Escape") {
      naming = false;
      name = "";
      folderError = "";
    }
  }

  function dragOver(e: DragEvent, id: string) {
    if (!store.dragging) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    over = id;
  }

  async function drop(e: DragEvent, id: string) {
    e.preventDefault();
    over = null;
    const item = store.dragging;
    store.dragging = null;
    if (!item) return;
    try {
      await store.changeFolders("move_to_folder", { kind: item.kind, id: item.id, folder: id });
    } catch (e) {
      folderError = why(e);
    }
  }

  const focus = (el: HTMLInputElement) => el.focus();

  // Home is on while it shows everything; a folder's row is on while it shows that.
  const isOn = (p: Page) => store.page === p && !tabs.shown && (p !== "home" || !store.folder);

  const badge = (p: Page): { n?: number; dot?: string } | null => {
    if (p === "home" && store.confirmations.length) return { dot: "wait" };
    if (p === "meetings" && store.recording) return { dot: "wrong" };
    if (p === "todos" && store.openTodos) return { n: store.openTodos };
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
          <span class="via">{onWifi(b.port) ? t.link.wifi : b.port ? t.link.usb : "…"}</span>
        </button>
      {/each}
    </div>
  {/if}

  <ul>
    {#each MAIN as p (p)}{@render item(p)}{/each}
  </ul>

  <div class="folders" role="group" aria-label={tf.title}>
    <div class="label">
      <span>{tf.title}</span>
      <button class="icon-btn small" title={tf.create} aria-label={tf.create} onclick={() => ((naming = true), (folderError = ""))}><Icon name="folder-plus" size={15} /></button>
    </div>
    <ul>
      {#each store.folders.folders as f (f.id)}
        {@const on = folderOn(f.id)}
        <li>
          <button
            class="nav-row folder"
            class:on
            class:over={over === f.id}
            aria-current={on ? "page" : undefined}
            title={store.dragging ? tf.dropHere(f.name) : f.name}
            onclick={() => store.openFolder(f.id)}
            ondragover={(e) => dragOver(e, f.id)}
            ondragleave={() => over === f.id && (over = null)}
            ondrop={(e) => drop(e, f.id)}>
            <Icon name="folder" size={16} filled={on} />
            <span class="fname">{f.name}</span>
            {#if inFolder(f.id)}<span class="fcount">{tf.count(inFolder(f.id))}</span>{/if}
          </button>
        </li>
      {/each}
      {#if naming}
        <li class="naming">
          <Icon name="folder" size={16} />
          <input type="text" bind:value={name} placeholder={tf.namePlaceholder} aria-label={tf.namePlaceholder} title={tf.nameHint} maxlength="60" onkeydown={nameKeys} onblur={() => void create()} use:focus />
        </li>
      {/if}
    </ul>
    {#if folderError}<p class="error-text ferror">{folderError}</p>{/if}
  </div>

  <footer>
    {#if pocket}
      <button class="nav-row device" title={t.pocket.openTitle} onclick={() => (store.pocketShown = true)}>
        {@render device("Pocket", pocket)}
      </button>
    {/if}
    {#if icloud}
      <button class="nav-row device" title={t.icloud.side.title} onclick={openICloud}>
        {@render device("iCloud", icloud)}
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

{#snippet device(name: string, d: { dot: string; text: string; by: "wifi" | "usb" | "bluetooth" | "cloud" | null })}
  <span class="dot {d.dot}"></span>
  <span class="dname">{name}</span>
  <span class="dstate">{d.text}{#if d.by}<Icon name={d.by} size={14} />{/if}</span>
{/snippet}

{#snippet item(p: Page)}
  {@const b = badge(p)}
  <li>
    <button class="nav-row" class:on={isOn(p)} aria-current={isOn(p) ? "page" : undefined} onclick={() => (p === "home" ? store.openFolder(null) : store.go(p))}>
      <Icon name={PAGE_ICONS[p]} filled={isOn(p)} />
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
  /* Folders: under the pages, scrolling on their own when there are many. */
  .folders {
    display: flex;
    flex-direction: column;
    min-height: 0;
    margin-top: 18px;
    padding-bottom: 12px;
  }
  .folders .label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 4px 4px 10px;
    color: var(--haze);
    font: 600 var(--fs-meta)/1 var(--sans);
  }
  .folders ul {
    overflow-y: auto;
    min-height: 0;
  }
  .folders .nav-row {
    gap: 11px;
    min-height: 34px;
  }
  .folders .nav-row.on {
    font-weight: 600;
  }
  /* An item dragged over: the folder it would go in. */
  .folders .nav-row.over {
    background: var(--hover);
    color: var(--pearl);
    box-shadow: inset 0 0 0 1.5px var(--pearl-deep);
  }
  .fname {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fcount {
    color: var(--dusk);
    font-size: var(--fs-small);
    font-variant-numeric: tabular-nums;
  }
  .naming {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 2px 4px 2px 10px;
    color: var(--haze);
  }
  .naming input {
    flex: 1;
    min-width: 0;
    height: 30px;
  }
  .ferror {
    margin: 6px 10px 0;
    font-size: var(--fs-small);
  }
  /* Pocket, iCloud and Yoru: the dot on the left, the state and its link on the right. */
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
