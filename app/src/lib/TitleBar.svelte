<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { monitor, type DeviceInfo } from "./monitor.svelte";
  import { MOODS } from "./petMood";
  import { i18n } from "./i18n/index.svelte";
  import { TABS } from "./settings/nav.svelte";
  import { issues, level } from "./settings/requirements";

  let { onSettings, onPet }: { onSettings: () => void; onPet: () => void } = $props();

  const t = $derived(i18n.t.main);
  const pet = $derived(
    monitor.pet
      ? { color: (MOODS[monitor.pet.mood] ?? MOODS.content).color, label: (i18n.t.pet.moods[monitor.pet.mood] ?? i18n.t.pet.moods.content).label, away: monitor.pet.away }
      : null,
  );

  const pill = $derived.by(() => {
    const port = monitor.port ?? "";
    if (monitor.setup) return { tone: "busy", text: t.pill.preparing, detail: monitor.setup.message };
    if (monitor.job === "flash") return { tone: "busy", text: t.pill.flashing(Math.round(monitor.flashPercent)), detail: "esptool" };
    if (monitor.job === "identify") return { tone: "busy", text: t.pill.identifying, detail: "esptool" };
    switch (monitor.boardState) {
      case "live":
        return { tone: "live", text: t.pill.live, detail: port };
      case "stale":
        return { tone: "stale", text: t.pill.stale, detail: t.pill.staleDetail };
      case "waiting":
        return { tone: "busy", text: t.pill.connected, detail: t.pill.connectedDetail };
      case "boot":
        return { tone: "busy", text: t.pill.booting, detail: port };
    }
    if (monitor.link === "offline") return { tone: "error", text: t.pill.disconnected, detail: monitor.message };
    return { tone: "busy", text: t.pill.searching, detail: t.pill.searchingDetail };
  });

  // Restarting the ping animation on every frame written makes the dot "beat" with the data.
  const beat = $derived(monitor.sentAt);

  // What the settings still need, on the settings icon: the worst as its dot, all of them on hover.
  const needs = $derived(TABS.flatMap((tab) => issues(tab).map((i) => ({ ...i, tab }))));
  const needsLevel = $derived(level(needs));
  const settingsTitle = $derived(
    needs.length ? [t.bar.settingsNeeds, ...needs.map((i) => `• ${i18n.t.settings.tabs[i.tab]}: ${i.text}`)].join("\n") : t.bar.settings,
  );

  const win = monitor.preview ? null : getCurrentWindow();

  // Several boards: a chip each, the one shown lit.
  const KIND_NAMES = { dualeye: "DualEye", yoru: "Yoru" } as const;
  const boardName = (b: DeviceInfo) => (b.kind ? KIND_NAMES[b.kind] : b.target.replace(/^.*\//, ""));
  const shortId = (b: DeviceInfo) => (b.id ? b.id.slice(-4) : "");
  const brand = $derived(monitor.boards.length > 1 ? "Companion Hub" : monitor.viewKind ? KIND_NAMES[monitor.viewKind] : "Companion Hub");
  const yoruLogo = $derived(monitor.boards.length <= 1 && monitor.viewKind === "yoru");
</script>

<header class="bar" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    {#if yoruLogo}
      <svg class="logo yoru" viewBox="0 0 40 20" aria-hidden="true">
        <defs>
          <radialGradient id="bar-yoru" cx="0.36" cy="0.3" r="0.78">
            <stop offset="0" stop-color="#f4f2fa" />
            <stop offset="0.55" stop-color="#c3bedb" />
            <stop offset="1" stop-color="#6e6890" />
          </radialGradient>
        </defs>
        <ellipse cx="7" cy="12.5" rx="3" ry="2.3" fill="url(#bar-yoru)" />
        <ellipse cx="33" cy="8" rx="3" ry="2.3" fill="url(#bar-yoru)" />
        <circle cx="20" cy="10" r="9" fill="url(#bar-yoru)" />
        <rect x="16.6" y="9" width="2" height="4.2" rx="1" fill="#17151f" />
        <rect x="21.4" y="9" width="2" height="4.2" rx="1" fill="#17151f" />
      </svg>
    {:else}
    <svg class="logo" viewBox="0 0 40 20" aria-hidden="true">
      <circle cx="9" cy="10" r="8" fill="none" stroke="var(--cpu)" stroke-width="2.4" stroke-dasharray="36 60" transform="rotate(-90 9 10)" stroke-linecap="round" />
      <circle cx="31" cy="10" r="8" fill="none" stroke="var(--gpu)" stroke-width="2.4" stroke-dasharray="24 60" transform="rotate(-90 31 10)" stroke-linecap="round" />
      <circle cx="9" cy="10" r="8" fill="none" stroke="rgba(255,255,255,.08)" stroke-width="2.4" />
      <circle cx="31" cy="10" r="8" fill="none" stroke="rgba(255,255,255,.08)" stroke-width="2.4" />
    </svg>
    {/if}
    <span class="name" data-tauri-drag-region>{brand}</span>
    {#if monitor.preview}<span class="badge" title={t.bar.previewTitle}>{t.bar.previewBadge}</span>{/if}
    {#if monitor.boards.length > 1}
      <div class="switcher" role="tablist" aria-label={t.bar.boards} data-tour="boards">
        {#each monitor.boards as b (b.target)}
          <button
            role="tab"
            class="chip"
            class:on={b.current}
            class:waiting={b.link !== "connected"}
            aria-selected={b.current}
            title={b.primary ? `${t.bar.showBoard(boardName(b))}\n${t.bar.primary}` : t.bar.showBoard(boardName(b))}
            onclick={() => monitor.showBoard(b.target)}
          >
            <span class="dot {b.kind ?? ''}"></span>
            <span>{boardName(b)}</span>
            {#if b.link !== "connected"}<span class="sub">{t.bar.searching}</span>{:else if shortId(b)}<span class="sub">{shortId(b)}</span>{/if}
            {#if b.primary}<span class="star" aria-hidden="true">★</span>{/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="pill {pill.tone}" data-tauri-drag-region>
    {#key beat}<span class="led"></span>{/key}
    <span class="text">{pill.text}</span>
    {#if pill.detail}<span class="detail">{pill.detail}</span>{/if}
  </div>

  <div class="actions">
    {#if pet}
      <button class="mood" style:--c={pet.color} onclick={onPet} title={t.bar.mood} data-tour="pet">
        <span class="face" aria-hidden="true"><i></i><i></i></span>
        <span>{pet.away ? t.bar.resting : pet.label}</span>
      </button>
    {/if}
    <button class="icon settings" data-tour="settings" onclick={onSettings} aria-label={settingsTitle} title={settingsTitle}>
      <svg viewBox="0 0 24 24"><path d="M4 7h10M18 7h2M4 17h4M12 17h8" /><circle cx="16" cy="7" r="2" /><circle cx="10" cy="17" r="2" /></svg>
      {#if needsLevel}<span class="ndot {needsLevel}"></span>{/if}
    </button>
    {#if win}
      <span class="sep"></span>
      <button class="icon" onclick={() => win.minimize()} aria-label={t.bar.minimize} title={t.bar.minimize}>
        <svg viewBox="0 0 24 24"><path d="M6 12h12" /></svg>
      </button>
      <button class="icon" onclick={() => win.toggleMaximize()} aria-label={t.bar.maximize} title={t.bar.maximize}>
        <svg viewBox="0 0 24 24"><rect x="6.5" y="6.5" width="11" height="11" rx="2.5" /></svg>
      </button>
      <button class="icon close" onclick={() => win.close()} aria-label={t.bar.hide} title={t.bar.hideTitle}>
        <svg viewBox="0 0 24 24"><path d="M7 7l10 10M17 7L7 17" /></svg>
      </button>
    {/if}
  </div>
</header>

<style>
  .bar {
    position: relative;
    z-index: 10;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    height: 52px;
    padding: 0 12px 0 18px;
    user-select: none;
    -webkit-user-select: none;
  }
  .brand {
    display: flex;
    white-space: nowrap;
    min-width: 0;
    align-items: center;
    gap: 10px;
  }
  .logo {
    width: 32px;
    height: 16px;
  }
  .name {
    font: 650 13.5px/1 var(--sans);
    letter-spacing: 0.02em;
  }
  .switcher {
    display: flex;
    gap: 4px;
    margin-left: 6px;
    padding: 3px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.035);
    border: 1px solid var(--line);
    overflow-x: auto;
    min-width: 0;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 9px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--faint);
    font: 550 11px/1 var(--sans);
    white-space: nowrap;
    cursor: pointer;
    transition:
      color 200ms,
      background 200ms;
  }
  .chip:hover {
    color: var(--dim);
  }
  .chip.on {
    color: var(--text);
    background: rgba(255, 255, 255, 0.08);
  }
  .chip.waiting {
    opacity: 0.6;
  }
  .chip .sub {
    font: 500 9.5px/1 var(--mono);
    color: var(--faint);
  }
  .chip .star {
    font-size: 9px;
    color: var(--stale);
  }
  .chip .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--faint);
  }
  .chip .dot.dualeye {
    background: var(--cpu);
  }
  .chip .dot.yoru {
    background: #8f7cff;
  }
  .badge {
    white-space: nowrap;
    font: 600 9.5px/1 var(--mono);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--stale);
    padding: 4px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--stale) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--stale) 25%, transparent);
  }

  .pill {
    --tone: var(--faint);
    display: flex;
    align-items: center;
    gap: 9px;
    height: 30px;
    padding: 0 14px 0 12px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.035);
    border: 1px solid var(--line);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
    font: 550 12px/1 var(--sans);
    max-width: 46vw;
    overflow: hidden;
  }
  .pill.live {
    --tone: #5ee38a;
  }
  .pill.stale {
    --tone: var(--stale);
  }
  .pill.busy {
    --tone: #8f96a3;
  }
  .pill.error {
    --tone: var(--hot);
  }
  .led {
    position: relative;
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--tone);
    box-shadow: 0 0 10px var(--tone);
  }
  .led::after {
    content: "";
    position: absolute;
    inset: -1px;
    border-radius: 50%;
    border: 1.5px solid var(--tone);
    animation: ping 1s cubic-bezier(0, 0.5, 0.4, 1) forwards;
  }
  .pill.busy .led {
    animation: breathe 1.6s ease-in-out infinite;
  }
  .pill.busy .led::after,
  .pill.error .led::after,
  .pill.stale .led::after {
    display: none;
  }
  @keyframes ping {
    from {
      transform: scale(1);
      opacity: 0.9;
    }
    to {
      transform: scale(3.2);
      opacity: 0;
    }
  }
  @keyframes breathe {
    50% {
      opacity: 0.3;
    }
  }
  .text {
    color: var(--text);
  }
  .detail {
    font: 500 11px/1 var(--mono);
    color: var(--faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .actions {
    justify-self: end;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .mood {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    margin-right: 6px;
    padding: 0 11px 0 9px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--c) 30%, transparent);
    background: color-mix(in srgb, var(--c) 10%, transparent);
    color: var(--text);
    font: 550 11.5px/1 var(--sans);
    cursor: pointer;
    transition:
      background 300ms,
      border-color 300ms;
  }
  .mood:hover {
    background: color-mix(in srgb, var(--c) 18%, transparent);
  }
  /* Two little eyes in the mood's colour, blinking now and then. */
  .face {
    display: flex;
    gap: 3px;
  }
  .face i {
    width: 5px;
    height: 7px;
    border-radius: 2px;
    background: var(--c);
    animation: blink 5s infinite;
  }
  @keyframes blink {
    0%,
    94%,
    100% {
      transform: scaleY(1);
    }
    97% {
      transform: scaleY(0.15);
    }
  }
  .sep {
    width: 1px;
    height: 16px;
    background: var(--line);
    margin: 0 6px;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 9px;
    background: transparent;
    color: var(--dim);
    cursor: pointer;
    transition:
      background 160ms,
      color 160ms;
  }
  .icon.settings {
    position: relative;
  }
  .ndot {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    box-shadow: 0 0 0 2px var(--bg);
  }
  .ndot.warn {
    background: var(--warm);
  }
  .ndot.error {
    background: var(--hot);
  }
  .icon:hover {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text);
  }
  .icon.close:hover {
    background: color-mix(in srgb, var(--hot) 80%, transparent);
    color: #fff;
  }
  .icon:focus-visible {
    outline: 1.5px solid var(--cpu);
    outline-offset: 1px;
  }
  .icon svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
</style>
