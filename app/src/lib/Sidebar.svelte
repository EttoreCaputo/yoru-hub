<!--
  The sidebar: Yoru's own small presence at the top (lit by what it's doing),
  with two boards or more the list of them to pick the one shown, the pages
  (Notifications and Settings are icons in the window bar),
  and "do not disturb" at the bottom.
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

  const tint = $derived(
    store.asking || store.voice === "thinking" ? "think" : store.voice === "speaking" ? "done" : store.confirmations.length ? "wait" : null,
  );
  const status = $derived.by(() => {
    if (store.boardState === "live") return { dot: "done", text: t.link.live };
    if (store.boardState === "boot") return { dot: "think", text: t.link.boot };
    if (store.link === "offline") return { dot: "wrong", text: t.link.offline };
    return { dot: "", text: t.link.searching };
  });
  const via = $derived(store.port?.startsWith("wifi:") ? t.link.wifi : store.port ? t.link.usb : "");
  const quietUntil = $derived(store.info?.quiet_until ?? null);
  const several = $derived(store.boards.length > 1);
  const boardDot = (link: string) => (link === "connected" ? "done" : link === "searching" ? "think" : "wrong");

  const badge = (p: Page): { n?: number; dot?: string } | null => {
    if (p === "home" && store.confirmations.length) return { dot: "wait" };
    if (p === "meetings" && store.recording) return { dot: "wrong" };
    return null;
  };
</script>

<nav class="side chrome" class:gone={!look.sidebar} aria-label="Yoru" inert={!look.sidebar}>
  <header data-tauri-drag-region>
    <svg class="mini {tint ?? ''}" viewBox="0 0 40 40" aria-hidden="true">
      <defs>
        <radialGradient id="side-pearl" cx="0.36" cy="0.32" r="0.75">
          <stop offset="0" stop-color="#f4f2fa" />
          <stop offset="0.55" stop-color="#c3bedb" />
          <stop offset="1" stop-color="#6e6890" />
        </radialGradient>
        <linearGradient id="side-tint" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0.45" stop-color="currentColor" stop-opacity="0" />
          <stop offset="1" stop-color="currentColor" stop-opacity="0.8" />
        </linearGradient>
      </defs>
      <circle class="halo" cx="20" cy="20" r="14" fill="currentColor" />
      <circle cx="20" cy="20" r="11" fill="url(#side-pearl)" />
      <circle class="tint" cx="20" cy="20" r="11" fill="url(#side-tint)" />
      <rect class="eye" x="16.6" y="19" width="2.2" height="5" rx="1.1" fill="#17151f" />
      <rect class="eye" x="21.2" y="19" width="2.2" height="5" rx="1.1" fill="#17151f" />
    </svg>
    <div data-tauri-drag-region>
      <div class="name" data-tauri-drag-region>Yoru{#if store.preview}<span class="preview" title={t.nav.previewTitle}>{t.nav.preview}</span>{/if}</div>
      <div class="status" data-tauri-drag-region><span class="dot {status.dot}"></span>{status.text}{#if via && store.boardState === "live"}<span class="faint"> · {via}</span>{/if}</div>
    </div>
  </header>

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
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px 22px;
  }
  .mini {
    width: 40px;
    height: 40px;
    color: transparent;
    flex-shrink: 0;
    transition: color 400ms;
  }
  .mini.think {
    color: var(--think);
  }
  .mini.done {
    color: var(--done);
  }
  .mini.wait {
    color: var(--wait);
  }
  .halo {
    opacity: 0.35;
    filter: blur(4px);
    animation: breathe 2.6s ease-in-out infinite;
  }
  .eye {
    transform-box: fill-box;
    transform-origin: 50% 50%;
    animation: blink 4.6s infinite;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 650 var(--fs-head)/1 var(--display);
    letter-spacing: -0.01em;
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
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
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
  @keyframes breathe {
    50% {
      opacity: 0.6;
    }
  }
  @keyframes blink {
    0%,
    94%,
    100% {
      transform: scaleY(1);
    }
    97% {
      transform: scaleY(0.1);
    }
  }
</style>
