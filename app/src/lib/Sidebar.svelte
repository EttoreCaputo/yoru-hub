<!--
  The sidebar: Yoru's own small presence at the top (lit by what it's doing),
  the pages, and "do not disturb" at the bottom.
-->
<script lang="ts">
  import { i18n } from "./i18n/index.svelte";
  import { store, type Page } from "./store.svelte";

  const t = $derived(i18n.t);

  const ICONS: Record<Page, string> = {
    today: "M3 18h18M7 18a5 5 0 0 1 10 0M12 6v2.5M5.6 10.6l1.6 1.6M18.4 10.6l-1.6 1.6",
    conversation: "M5 6h14v9H10l-4 3v-3H5z",
    inbox: "M4 13h4l1.5 2.5h5L16 13h4M5.5 6h13L20 13v5H4v-5z",
    connectors: "M9 7V4M15 7V4M7 7h10v4a5 5 0 0 1-10 0zM12 16v4",
    routines: "M12 7v5l3 2M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17Z",
    memory: "M6 4h9l3 3v13H6zM9 10h6M9 14h6",
    yoru: "M12 5.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13ZM10.5 11v2M13.5 11v2",
    settings: "M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M10 15v4",
  };
  const MOON = "M4 13a8 8 0 0 0 15 3.5A8 8 0 0 1 9 5a8 8 0 0 0-5 8Z";
  const MAIN: Page[] = ["today", "conversation", "inbox", "connectors", "routines", "memory"];
  const BODY: Page[] = ["yoru", "settings"];

  const tint = $derived(
    store.asking || store.voice === "thinking" ? "think" : store.voice === "speaking" ? "done" : store.yoruApprovals.length || store.codeApprovals.length ? "wait" : null,
  );
  const status = $derived.by(() => {
    if (store.boardState === "live") return { dot: "done", text: t.link.live };
    if (store.boardState === "boot") return { dot: "think", text: t.link.boot };
    if (store.link === "offline") return { dot: "wrong", text: t.link.offline };
    return { dot: "", text: t.link.searching };
  });
  const via = $derived(store.port?.startsWith("wifi:") ? t.link.wifi : store.port ? t.link.usb : "");
  const quietUntil = $derived(store.info?.quiet_until ?? null);

  const badge = (p: Page): { n?: number; dot?: string } | null => {
    if (p === "inbox" && store.unread) return { n: store.unread };
    if (p === "conversation" && store.yoruApprovals.length) return { dot: "wait" };
    if (p === "yoru" && store.update) return { dot: "wait" };
    if (p === "settings" && store.info && !store.info.ready) return { dot: "wrong" };
    if (p === "connectors" && store.connectors.some((c) => c.enabled && (c.state === "needs_login" || c.state === "error"))) return { dot: "wait" };
    return null;
  };
</script>

<nav class="side chrome" aria-label="Yoru">
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

  <ul>
    {#each MAIN as p (p)}{@render item(p)}{/each}
  </ul>
  <ul class="body">
    {#each BODY as p (p)}{@render item(p)}{/each}
  </ul>

  <footer>
    {#if quietUntil}
      <div class="dnd on">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d={MOON} /></svg>
        <span>{t.dnd.on(i18n.time(quietUntil))}</span>
        <button class="btn quiet small" onclick={() => store.doNotDisturb(0)}>{t.dnd.end}</button>
      </div>
    {:else}
      <button class="dnd" onclick={() => store.doNotDisturb(60)} title={t.dnd.hour}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d={MOON} /></svg>
        <span>{t.dnd.off}</span>
        <span class="faint hour">{t.dnd.hour}</span>
      </button>
    {/if}
  </footer>
</nav>

{#snippet item(p: Page)}
  {@const b = badge(p)}
  <li>
    <button class:on={store.page === p} aria-current={store.page === p ? "page" : undefined} onclick={() => store.go(p)}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d={ICONS[p]} /></svg>
      <span>{t.nav[p]}</span>
      {#if b?.n}<span class="count">{b.n}</span>{:else if b?.dot}<span class="dot {b.dot}"></span>{/if}
    </button>
  </li>
{/snippet}

<style>
  .side {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 0 10px 12px;
    background: var(--ink);
    border-right: 1px solid var(--rim);
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 22px 8px 22px;
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
    font: 650 19px/1 var(--display);
    letter-spacing: -0.01em;
  }
  .preview {
    padding: 3px 6px;
    border-radius: 6px;
    background: var(--ink-3);
    color: var(--haze);
    font: 600 9.5px/1 var(--sans);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    color: var(--haze);
    font-size: 12px;
    white-space: nowrap;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }
  ul.body {
    margin-top: 14px;
    padding-top: 14px;
    border-top: 1px solid var(--rim);
  }
  li button {
    display: flex;
    align-items: center;
    gap: 11px;
    width: 100%;
    height: 36px;
    padding: 0 10px;
    border: 0;
    border-radius: 10px;
    background: none;
    color: var(--haze);
    font: 500 13.5px/1 var(--sans);
    cursor: pointer;
    text-align: left;
    transition:
      background 140ms,
      color 140ms;
  }
  li button:hover {
    color: var(--pearl);
    background: var(--ink-2);
  }
  li button.on {
    color: var(--pearl);
    background: var(--ink-3);
  }
  li span:first-of-type {
    flex: 1;
  }
  svg:not(.mini) {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex-shrink: 0;
  }
  .count {
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--pearl);
    color: var(--eye);
    font: 700 11px/20px var(--sans);
    text-align: center;
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
    border-radius: 10px;
    border: 1px solid var(--rim);
    background: none;
    color: var(--haze);
    font: 500 12.5px/1.2 var(--sans);
    cursor: pointer;
    text-align: left;
  }
  .dnd:hover {
    color: var(--pearl);
  }
  .dnd .hour {
    margin-left: auto;
    font-size: 11.5px;
  }
  .dnd.on {
    cursor: default;
    color: var(--pearl);
    border-color: var(--rim-2);
    background: var(--ink-2);
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
