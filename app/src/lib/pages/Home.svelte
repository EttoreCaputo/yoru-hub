<!--
  Home, after Pocket's: every conversation with Yoru and every meeting
  recorded, newest first, by day, with a search over their titles; each
  opens in a tab of its own (tabs.svelte.ts). While the Pocket syncs, the
  recordings it is copying head the list, with how far the copy has got; a
  click opens the Pocket's dialog. With a folder picked in the sidebar
  (store.folder), only what's in it, with its name, rename and delete; a
  row goes in a folder from its folder button or dragged onto the folder
  in the sidebar. At the bottom the field to ask
  Yoru something, which starts a new conversation in a new tab. With
  nothing yet, Yoru greets you in the middle.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import { tabs } from "../tabs.svelte";
  import type { Meeting } from "../types";
  import Composer from "../ui/Composer.svelte";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import ConfirmCard from "../ui/ConfirmCard.svelte";
  import Icon from "../ui/Icon.svelte";
  import YoruBoard from "../YoruBoard.svelte";

  const t = $derived(i18n.t.home);
  const tm = $derived(i18n.t.meetings);
  const tf = $derived(i18n.t.folders);
  /** The folder shown (null: everything). */
  const shown = $derived(store.folders.folders.find((f) => f.id === store.folder) ?? null);
  const folderName = (id: string | null) => store.folders.folders.find((f) => f.id === id)?.name ?? "";
  let query = $state("");
  let error = $state("");

  type Row = {
    kind: "chat" | "meeting" | "pocket";
    id: string;
    title: string;
    faint: boolean;
    note: string;
    at: number;
    now: boolean;
    folder: string | null;
    /** A Pocket recording being copied (0 to 1), or waiting its turn (null). */
    progress?: number | null;
  };

  const meetingTitle = (m: Meeting) => m.title ?? tm.untitled(`${i18n.date(m.started_at)} · ${i18n.time(m.started_at)}`);
  const rows = $derived.by<Row[]>(() => {
    const chats: Row[] = store.conversations.map((c) => ({
      kind: "chat",
      id: c.id,
      title: c.title || c.preview || t.untitled,
      faint: !c.title,
      note: c.title ? c.preview : "",
      at: c.updated,
      now: c.id === store.currentChat,
      folder: store.folderOf("chat", c.id),
    }));
    const meetings: Row[] = store.meetings.map((m) => ({
      kind: "meeting",
      id: m.id,
      title: meetingTitle(m),
      faint: false,
      note: [m.summarized_at ? t.hasSummary : "", m.transcribed_at ? t.hasTranscript : ""].filter(Boolean).join(" · "),
      at: m.started_at,
      now: m.state === "recording",
      folder: store.folderOf("meeting", m.id),
    }));
    const q = query.trim().toLowerCase();
    return [...chats, ...meetings]
      .filter((r) => !shown || r.folder === shown.id)
      .filter((r) => !q || `${r.title} ${r.note}`.toLowerCase().includes(q))
      .sort((a, b) => b.at - a.at);
  });

  // The Pocket's recordings being copied: the one under way, then those waiting.
  const copying = $derived.by<Row[]>(() => {
    const tr = store.pocket?.transfer;
    if (!tr || shown) return [];
    return [tr.name, ...tr.waiting].map((name) => {
      const at = store.pocket?.recordings.find((r) => r.name === name)?.started_at ?? store.now;
      const progress = name === tr.name ? (tr.total ? Math.min(1, tr.bytes / tr.total) : 0) : null;
      const note = progress === null ? t.copyWaiting : tr.wifi ? t.copyingWifi : t.copying;
      return { kind: "pocket", id: name, title: tm.untitled(`${i18n.date(at)} · ${i18n.time(at)}`), faint: false, note, at, now: true, folder: null, progress };
    });
  });

  // By day: Today, Yesterday, then the weekday and date.
  const days = $derived.by(() => {
    const groups: { key: string; name: string; date: string; rows: Row[] }[] = [];
    const today = new Date(store.now).toDateString();
    const yesterday = new Date(store.now - 86_400_000).toDateString();
    for (const r of rows) {
      const key = new Date(r.at).toDateString();
      let g = groups.at(-1);
      if (g?.key !== key) {
        const date = i18n.date(r.at, { weekday: "long", day: "numeric", month: "long" });
        const name = key === today ? t.today : key === yesterday ? t.yesterday : "";
        g = { key, name: name || date, date: name ? date : "", rows: [] };
        groups.push(g);
      }
      g.rows.push(r);
    }
    return copying.length ? [{ key: "pocket", name: t.pocketKind, date: "", rows: copying }, ...groups] : groups;
  });

  const nothing = $derived(!shown && !store.conversations.length && !store.meetings.length && !store.pocket?.transfer);
  const inShown = $derived(shown ? store.folderCount(shown.id) : 0);
  const ready = $derived(store.info?.ready ?? true);
  const greeting = $derived.by(() => {
    const h = new Date(store.now).getHours();
    return h < 5 ? t.greeting.night : h < 12 ? t.greeting.morning : h < 18 ? t.greeting.afternoon : h < 23 ? t.greeting.evening : t.greeting.night;
  });
  let reaction = $state<{ name: string; at: number } | null>(null);
  $effect(() => {
    const hello = setTimeout(() => (reaction = { name: "wave", at: Date.now() }), 500);
    return () => clearTimeout(hello);
  });

  const open = (r: Row) => (r.kind === "pocket" ? (store.pocketShown = true) : tabs.open({ kind: r.kind, id: r.id }));
  const kindName = (r: Row) => (r.kind === "chat" ? t.chatKind : r.kind === "pocket" ? t.pocketKind : t.meetingKind);

  async function remove(r: Row) {
    error = "";
    try {
      if (r.kind === "chat") await call("delete_conversation", { id: r.id });
      else {
        await call("delete_meeting", { id: r.id });
        await store.refreshMeetings();
      }
    } catch (e) {
      error = why(e);
    }
  }

  // A new conversation started inside a folder goes in it.
  async function newChat() {
    const id = await store.newChat();
    if (shown) await store.changeFolders("move_to_folder", { kind: "chat", id, folder: shown.id }).catch(() => {});
  }

  // The folder shown: renamed in place, or deleted (its items stay).
  let renaming = $state(false);
  let newName = $state("");
  function startRename() {
    newName = shown?.name ?? "";
    renaming = true;
  }
  async function saveRename() {
    if (!renaming || !shown) return;
    renaming = false;
    const n = newName.trim();
    if (!n || n === shown.name) return;
    error = "";
    try {
      await store.changeFolders("rename_folder", { id: shown.id, name: n });
    } catch (e) {
      error = why(e);
    }
  }
  function renameKeys(e: KeyboardEvent) {
    if (e.key === "Enter") void saveRename();
    else if (e.key === "Escape") renaming = false;
  }
  async function deleteFolder() {
    if (!shown) return;
    error = "";
    try {
      await store.changeFolders("delete_folder", { id: shown.id });
    } catch (e) {
      error = why(e);
    }
  }
  $effect(() => {
    // Another folder picked: whatever was being renamed is dropped.
    void store.folder;
    renaming = false;
  });

  // A row's folder menu, and dragging a row onto a folder in the sidebar.
  let menuFor = $state<string | null>(null);
  const key = (r: Row) => r.kind + r.id;
  async function moveTo(r: Row, folder: string | null) {
    menuFor = null;
    if (r.kind === "pocket") return;
    error = "";
    try {
      await store.changeFolders("move_to_folder", { kind: r.kind, id: r.id, folder });
    } catch (e) {
      error = why(e);
    }
  }
  function dragStart(e: DragEvent, r: Row) {
    if (r.kind === "pocket" || !e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", r.title);
    store.dragging = { kind: r.kind, id: r.id };
    menuFor = null;
  }
  const focus = (el: HTMLInputElement) => {
    el.focus();
    el.select();
  };

  let search: HTMLInputElement | undefined = $state();
  // ⌘K finds, as in Pocket.
  function keys(e: KeyboardEvent) {
    if (e.key === "Escape") menuFor = null;
    if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      search?.focus();
    }
  }
</script>

<svelte:window onkeydown={keys} onclick={() => (menuFor = null)} />

<section class="home">
  <div class="scroll">
    <div class="page">
      <div class="page-head">
        <div>
          {#if shown && renaming}
            <input class="rename" type="text" bind:value={newName} aria-label={tf.rename} title={tf.nameHint} maxlength="60" onkeydown={renameKeys} onblur={() => void saveRename()} use:focus />
          {:else if shown}
            <h1 class="folder-title"><Icon name="folder" size={22} />{shown.name}</h1>
          {:else}
            <h1>{t.all}</h1>
          {/if}
          <p class="lede">{shown ? tf.lede(inShown) : t.allLede}</p>
        </div>
        <div class="actions">
          {#if shown}
            <button class="icon-btn" title={tf.rename} aria-label={tf.rename} onclick={startRename}><Icon name="pencil" size={17} /></button>
            <ConfirmButton icon="trash-2" label={tf.delete} question={tf.deleteConfirm} sure={tf.deleteSure} onconfirm={deleteFolder} />
          {/if}
          <button class="btn primary" onclick={newChat}><Icon name="plus" size={15} stroke={2.25} />{t.newChat}</button>
        </div>
      </div>

      {#if store.confirmations.length}
        <div class="cards">
          {#each store.confirmations as c (c.id)}<ConfirmCard card={c} />{/each}
        </div>
      {/if}

      {#if nothing}
        <div class="mascot">
          <div class="yoru"><YoruBoard board="live" size={130} {reaction} follow onpoke={() => (reaction = { name: "happy", at: Date.now() })} /></div>
          <p class="eyebrow">{greeting}</p>
          <h2 class="said">{t.noneYet}</h2>
          <p class="dim">{t.noneYetHint}</p>
          {#if ready}
            <div class="suggestions">
              {#each t.suggestions as s (s)}<button class="btn" onclick={() => store.askNew(s)}>{s}</button>{/each}
            </div>
          {:else}
            <button class="btn primary" onclick={() => store.go("settings")}>{t.noKeyAction}</button>
          {/if}
        </div>
      {:else if shown && !inShown}
        {#if error}<p class="error-text">{error}</p>{/if}
        <div class="mascot">
          <span class="empty-icon"><Icon name="folder" size={28} /></span>
          <h2 class="said">{tf.empty}</h2>
          <p class="dim">{tf.emptyHint}</p>
        </div>
      {:else}
        <label class="search">
          <Icon name="search" size={16} />
          <input type="search" bind:this={search} bind:value={query} placeholder={t.search} aria-label={t.search} />
          <kbd>⌘K</kbd>
        </label>
        {#if error}<p class="error-text">{error}</p>{/if}

        {#each days as day (day.key)}
          <h2 class="day">{day.name}{#if day.date}<span>{day.date}</span>{/if}</h2>
          <ul class="rows">
            {#each day.rows as r (r.kind + r.id)}
              <li class:removable={r.kind !== "pocket" && (!r.now || store.folders.folders.length > 0)} class:menu-open={menuFor === key(r)} draggable={r.kind !== "pocket"} ondragstart={(e) => dragStart(e, r)} ondragend={() => (store.dragging = null)}>
                <button class="row" onclick={() => open(r)}>
                  <span class="kind {r.kind}" title={kindName(r)}>
                    <Icon name={r.kind === "chat" ? "message-circle" : r.kind === "pocket" ? "audio-lines" : "file-text"} size={16} />
                  </span>
                  <span class="what">
                    <span class="name" class:faint={r.faint}>{r.title}</span>
                    {#if r.note}<span class="note">{r.note}</span>{/if}
                    {#if typeof r.progress === "number"}<span class="meter"><span style:--p={r.progress}></span></span>{/if}
                  </span>
                  {#if r.kind === "pocket"}
                    {#if typeof r.progress === "number"}<span class="tag done percent">{Math.round(r.progress * 100)}%</span>{/if}
                  {:else if r.now}<span class="tag done">{t.now}</span>{/if}
                  {#if !shown && r.folder}<span class="in-folder" title={folderName(r.folder)}><Icon name="folder" size={13} />{folderName(r.folder)}</span>{/if}
                  <time class="faint">{i18n.time(r.at)}</time>
                </button>
                {#if r.kind !== "pocket"}
                  <span class="remove">
                    {#if store.folders.folders.length}
                      <button class="icon-btn" class:on={menuFor === key(r)} aria-label={tf.moveTo} title={tf.moveTo} aria-haspopup="menu" aria-expanded={menuFor === key(r)} onclick={(e) => { e.stopPropagation(); menuFor = menuFor === key(r) ? null : key(r); }}><Icon name="folder" size={17} /></button>
                    {/if}
                    {#if !r.now}<ConfirmButton icon="trash-2" small label={r.kind === "chat" ? t.forget : tm.delete} question={r.kind === "chat" ? t.forgetConfirm : tm.deleteConfirm} sure={r.kind === "chat" ? t.forgetSure : tm.delete} onconfirm={() => remove(r)} />{/if}
                    {#if menuFor === key(r)}
                      <div class="menu" role="menu" aria-label={tf.moveTo}>
                        {#each store.folders.folders as f (f.id)}
                          <button role="menuitemradio" aria-checked={r.folder === f.id} onclick={(e) => { e.stopPropagation(); void moveTo(r, f.id); }}>
                            <Icon name="folder" size={15} /><span>{f.name}</span>{#if r.folder === f.id}<Icon name="check" size={15} />{/if}
                          </button>
                        {/each}
                        {#if r.folder}
                          <hr />
                          <button role="menuitem" onclick={(e) => { e.stopPropagation(); void moveTo(r, null); }}><Icon name="x" size={15} /><span>{tf.none}</span></button>
                        {/if}
                      </div>
                    {/if}
                  </span>
                {/if}
              </li>
            {/each}
          </ul>
        {/each}
        {#if !rows.length && !copying.length}
          <p class="faint none">{t.noMatch}</p>
        {/if}
      {/if}
    </div>
  </div>

  <div class="dock">
    <Composer mic conversation="new" />
  </div>
</section>

<style>
  .home {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .page {
    padding-bottom: 24px;
  }
  .cards {
    display: grid;
    gap: 10px;
    margin-bottom: 20px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 8px 0 14px;
    border: 1px solid var(--rim-2);
    border-radius: 999px;
    background: var(--field);
    color: var(--dusk);
    transition: border-color 160ms;
  }
  .search:focus-within {
    border-color: var(--pearl-deep);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: none;
    box-shadow: none;
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  kbd {
    padding: 2px 6px;
    border: 1px solid var(--rim-2);
    border-radius: 6px;
    color: var(--dusk);
    font: 500 var(--fs-small)/1.2 var(--sans);
  }
  .day {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 28px 0 8px;
    font: 600 var(--fs-body)/1.3 var(--sans);
    color: var(--pearl);
  }
  .day span {
    color: var(--dusk);
    font-weight: 400;
    font-size: var(--fs-meta);
  }
  .rows {
    list-style: none;
    margin: 0 -10px;
    padding: 0;
  }
  .rows li {
    position: relative;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-radius: var(--r-md);
    background: none;
    color: var(--pearl);
    text-align: left;
    cursor: pointer;
    transition: background 140ms;
  }
  .row:hover,
  .row:focus-visible {
    background: var(--hover);
  }
  /* A document's icon, as Pocket draws a recording: a small sheet. */
  .kind {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 30px;
    height: 34px;
    border: 1px solid var(--rim-2);
    border-radius: 7px;
    background: var(--night);
    color: var(--dusk);
  }
  .kind.pocket {
    color: var(--pearl);
  }
  .what .meter {
    margin-top: 4px;
    max-width: 240px;
  }
  .percent {
    font-variant-numeric: tabular-nums;
  }
  .kind.chat {
    border-radius: 50%;
    width: 34px;
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .name,
  .note {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-size: var(--fs-body);
    font-weight: 500;
  }
  .note {
    color: var(--dusk);
    font-size: var(--fs-meta);
  }
  time {
    flex-shrink: 0;
    font-size: var(--fs-meta);
    font-variant-numeric: tabular-nums;
  }
  /* Delete shows over the time, on the row pointed at. */
  .remove {
    position: absolute;
    right: 8px;
    top: 50%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-left: 8px;
    transform: translateY(-50%);
    opacity: 0;
    transition: opacity 140ms;
  }
  .rows li:hover .remove,
  .remove:focus-within {
    opacity: 1;
  }
  .rows li:hover .row {
    background: var(--hover);
  }
  .rows li.removable:hover time {
    visibility: hidden;
  }
  .none {
    margin-top: 28px;
  }
  .folder-title {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .folder-title :global(.icon) {
    color: var(--haze);
  }
  .rename {
    width: min(420px, 100%);
    font: 600 var(--fs-title)/1.2 var(--display);
  }
  .empty-icon {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    margin-bottom: 10px;
    border: 1px solid var(--rim-2);
    border-radius: var(--r);
    background: var(--ink-2);
    color: var(--haze);
  }
  /* In the full list, the folder a row is in. */
  .in-folder {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    max-width: 160px;
    overflow: hidden;
    color: var(--dusk);
    font-size: var(--fs-meta);
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .rows li[draggable="true"] {
    -webkit-user-drag: element;
  }
  /* Over the rows below it, which its menu covers. */
  .rows li.menu-open {
    z-index: 5;
  }
  .rows li.menu-open .remove {
    opacity: 1;
  }
  .rows li.menu-open .row {
    background: var(--hover);
  }
  .rows li.removable:hover .in-folder,
  .rows li.menu-open .in-folder,
  .rows li.menu-open time {
    visibility: hidden;
  }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 10;
    display: grid;
    min-width: 200px;
    max-height: 280px;
    overflow-y: auto;
    padding: 5px;
    border: 1px solid var(--rim-2);
    border-radius: var(--r);
    background: var(--ink);
    box-shadow: var(--lift);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 7px 9px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    color: var(--pearl);
    font: 500 var(--fs-meta)/1.3 var(--sans);
    text-align: left;
    cursor: pointer;
  }
  .menu button:hover,
  .menu button:focus-visible {
    background: var(--hover);
  }
  .menu button span {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu button :global(.icon) {
    color: var(--haze);
  }
  .menu hr {
    margin: 4px 2px;
    border: 0;
    border-top: 1px solid var(--rim);
  }
  .mascot {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 24px 0 8px;
    text-align: center;
  }
  .yoru :global(svg > circle:first-of-type) {
    fill: transparent;
    stroke: transparent;
  }
  .eyebrow {
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .said {
    margin: 4px 0 6px;
    font: 600 var(--fs-head)/1.35 var(--display);
  }
  .suggestions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 18px;
  }
  .mascot > .btn {
    margin-top: 14px;
  }
  .dock {
    width: min(560px, calc(100% - 48px));
    margin: 0 auto;
    padding: 8px 0 20px;
  }
</style>
