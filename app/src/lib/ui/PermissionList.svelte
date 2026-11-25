<!--
  The Mac's permissions, as one panel of settings: each row its icon, what
  it's for, where it stands and the button that asks (macOS's prompt, again
  after a no; System Settings for one a profile holds). The microphone is
  required; the others are for meetings. Read again every second while
  shown, so a switch flipped in System Settings shows here at once; a
  permission just given lights up (`ongrant` tells the first run).
-->
<script lang="ts">
  import { call } from "../api";
  import type { IconName } from "../icons/icons";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { PermissionKind } from "../types";
  import Icon from "./Icon.svelte";

  /** In Settings, a granted one can be changed too (macOS turns it off, not the app). */
  /** `hints`: a shorter line per permission (the first run's). */
  let { manage = false, hints, ongrant }: { manage?: boolean; hints?: Record<PermissionKind, string>; ongrant?: (kind: PermissionKind) => void } = $props();

  const KINDS: PermissionKind[] = ["microphone", "calendar", "system_audio"];
  const ICON: Record<PermissionKind, IconName> = { microphone: "mic", calendar: "calendar", system_audio: "volume" };
  const t = $derived(i18n.t.permissions);
  let asking = $state<PermissionKind | null>(null);
  /** Those granted while shown: they play their light once. */
  let fresh = $state<Set<PermissionKind>>(new Set());
  let seen: Partial<Record<PermissionKind, string>> = {};

  $effect(() => {
    void store.refreshPermissions();
    const timer = setInterval(() => void store.refreshPermissions(), 1000);
    return () => clearInterval(timer);
  });

  $effect(() => {
    const now = store.permissions;
    if (!now) return;
    for (const kind of KINDS) {
      const was = seen[kind];
      if (was && was !== "granted" && now[kind] === "granted") {
        fresh = new Set([...fresh, kind]);
        ongrant?.(kind);
      }
      seen[kind] = now[kind];
    }
  });

  async function ask(kind: PermissionKind) {
    asking = kind;
    try {
      await store.requestPermission(kind);
    } finally {
      asking = null;
    }
  }
</script>

<section class="panel perms">
  {#each KINDS as kind, i (kind)}
    {@const state = store.permissions?.[kind] ?? "unknown"}
    {#if state !== "unsupported"}
      <div class="setting" class:granted={state === "granted"} class:fresh={fresh.has(kind)} style:--i={i}>
        <span class="mark" aria-hidden="true">
          <Icon name={ICON[kind]} size={18} />
          {#if state === "granted"}
            <svg class="tick" viewBox="0 0 16 16"><circle cx="8" cy="8" r="8" /><path d="m4.6 8.2 2.2 2.2 4.6-4.6" /></svg>
          {/if}
        </span>
        <div class="what">
          <b>{t[kind].name}{#if kind === "microphone"}<span class="tag outline req">{t.required}</span>{/if}</b>
          <span>{hints?.[kind] ?? t[kind].why}</span>
          {#if state === "denied" || state === "restricted"}<span class="off">{t.denied}</span>{/if}
        </div>
        <div class="how">
          {#if state === "granted"}
            <span class="tag done ok">{t.granted}</span>
            {#if manage}<button class="btn quiet small" onclick={() => call("open_privacy", { kind }).catch(() => {})}>{t.change}</button>{/if}
          {:else}
            <button class="btn small" class:primary={kind === "microphone"} disabled={!!asking} onclick={() => ask(kind)}>
              {asking === kind ? t.asking : state === "restricted" ? t.openSettings : state === "denied" ? t.askAgain : t.allow}
            </button>
          {/if}
        </div>
      </div>
    {/if}
  {/each}
</section>

<style>
  .setting {
    justify-content: flex-start;
    gap: 14px;
  }
  .what {
    flex: 1;
  }
  /* Beside the name, not under it like the row's other lines. */
  .what b .req {
    display: inline-flex;
    margin: 0 0 0 8px;
    vertical-align: 1px;
  }
  .what .off {
    color: var(--wait);
  }

  /* The permission's icon in a soft square; given, it turns green and a
     tick sits on its corner. */
  .mark {
    position: relative;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    border-radius: var(--r-md);
    background: var(--ink-3);
    color: var(--haze);
    transition:
      background 0.4s ease,
      color 0.4s ease;
  }
  .granted .mark {
    background: color-mix(in srgb, var(--done) 13%, transparent);
    color: var(--done);
  }
  .tick {
    position: absolute;
    right: -5px;
    bottom: -5px;
    width: 16px;
    height: 16px;
  }
  .tick circle {
    fill: var(--done);
    stroke: var(--ink);
    stroke-width: 2.5px;
    paint-order: stroke;
  }
  .tick path {
    fill: none;
    stroke: var(--ink);
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* Just given: the square pops, a ring spreads from it, the tick draws
     itself and the pill slides in. */
  .fresh .mark {
    animation: pop 0.6s cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  .fresh .mark::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    box-shadow: 0 0 0 0 var(--done);
    animation: ring 0.9s ease-out;
  }
  .fresh .tick {
    animation: tick-in 0.45s 0.15s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }
  .fresh .tick path {
    stroke-dasharray: 12;
    animation: draw 0.35s 0.35s ease-out both;
  }
  .fresh .ok {
    animation: slide 0.4s 0.2s cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }

  @keyframes pop {
    0% {
      transform: scale(1);
    }
    35% {
      transform: scale(0.86);
    }
    100% {
      transform: scale(1);
    }
  }
  @keyframes ring {
    from {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--done) 55%, transparent);
    }
    to {
      box-shadow: 0 0 0 14px transparent;
    }
  }
  @keyframes tick-in {
    from {
      transform: scale(0);
    }
  }
  @keyframes draw {
    from {
      stroke-dashoffset: 12;
    }
  }
  @keyframes slide {
    from {
      opacity: 0;
      transform: translateX(8px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .fresh .mark,
    .fresh .mark::after,
    .fresh .tick,
    .fresh .tick path,
    .fresh .ok {
      animation: none;
    }
  }
</style>
