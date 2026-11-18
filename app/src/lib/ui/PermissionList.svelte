<!--
  The Mac's permissions, one row each: what it's for, where it stands, and
  the button that asks (macOS's prompt the first time, System Settings once
  turned down). The microphone is required; the others are for meetings.
  Read again every second while shown, so a switch flipped in System
  Settings shows here at once.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { PermissionKind } from "../types";

  const KINDS: PermissionKind[] = ["microphone", "calendar", "system_audio"];
  const t = $derived(i18n.t.permissions);
  let asking = $state<PermissionKind | null>(null);

  $effect(() => {
    void store.refreshPermissions();
    const timer = setInterval(() => void store.refreshPermissions(), 1000);
    return () => clearInterval(timer);
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

<ul class="perms">
  {#each KINDS as kind (kind)}
    {@const state = store.permissions?.[kind] ?? "unknown"}
    {#if state !== "unsupported"}
      <li class:granted={state === "granted"}>
        <div class="what">
          <b>{t[kind].name}{#if kind === "microphone"}<span class="req">{t.required}</span>{/if}</b>
          <span>{t[kind].why}</span>
          {#if state === "denied" || state === "restricted"}<span class="off">{t.denied}</span>{/if}
        </div>
        {#if state === "granted"}
          <span class="ok"><span class="dot done"></span>{t.granted}</span>
        {:else}
          <button class="btn small" class:primary={kind === "microphone"} disabled={!!asking} onclick={() => ask(kind)}>
            {asking === kind ? t.asking : state === "unknown" ? t.allow : t.openSettings}
          </button>
        {/if}
      </li>
    {/if}
  {/each}
</ul>

<style>
  .perms {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 14px;
    border-radius: var(--r);
    border: 1px solid var(--rim);
    background: var(--ink-2);
  }
  li.granted {
    border-color: color-mix(in srgb, var(--done) 40%, transparent);
  }
  .what {
    flex: 1;
    display: grid;
    gap: 2px;
  }
  .what span {
    color: var(--haze);
    font-size: 12.5px;
  }
  .what .off {
    color: var(--wait);
  }
  .req {
    margin-left: 8px;
    padding: 1px 7px;
    border-radius: 999px;
    border: 1px solid var(--rim-2);
    color: var(--pearl);
    font: 600 10.5px/1.5 var(--sans);
    vertical-align: 1px;
  }
  .ok {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--haze);
    font-size: 12.5px;
    white-space: nowrap;
  }
</style>
