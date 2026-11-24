<!--
  Settings > Pocket (docs/pocket.md): the recorder as it is now (its
  dialog, from the sidebar too, has the sync and the pairing), then
  whether Yoru keeps it connected, how recordings come over, and the
  device itself.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { PocketSettings } from "../types";
  import ConfirmButton from "./ConfirmButton.svelte";
  import Toggle from "./Toggle.svelte";

  const t = $derived(i18n.t.pocket);
  const p = $derived(store.pocket);
  let error = $state("");

  async function save(change: Partial<PocketSettings>) {
    if (!p) return;
    error = "";
    try {
      await call("set_pocket_settings", { settings: { ...p.settings, ...change } });
    } catch (e) {
      error = why(e);
    }
  }
  const forget = () => call("pocket_forget").catch((e) => (error = why(e)));

  const dot = $derived(!p ? "" : p.phase === "connected" ? "done" : p.phase === "refused" || p.phase === "no_bluetooth" ? "wrong" : p.phase === "unpaired" || p.phase === "off" ? "" : "think");
</script>

{#if !p}
  <p class="dim">{t.unavailable}</p>
{:else}
  <section class="panel">
    <div class="setting">
      <div class="what">
        <b>{p.name ?? "Pocket"}</b>
        <span class="state"><span class="dot {dot}"></span>{t.phases[p.phase] + (p.phase === "connected" && p.battery !== null ? ` · ${p.battery}%` : "")}</span>
      </div>
      <div class="how">
        {#if p.phase !== "off"}
          <button class="btn small" class:primary={p.phase === "unpaired" || p.phase === "refused"} onclick={() => (store.pocketShown = true)}>
            {p.phase === "unpaired" || p.phase === "refused" ? t.pair : t.showDevice}
          </button>
        {/if}
      </div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.enabled}</b><span>{t.enabledHint}</span></div>
      <div class="how"><Toggle checked={p.settings.enabled} label={t.enabled} onchange={(on) => save({ enabled: on })} /></div>
    </div>
  </section>

  {#if p.settings.enabled}
    <h2 class="section-title">{t.stepSync}</h2>
    <p class="dim note">{t.stepSyncNote}</p>
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.autoSync}</b><span>{t.autoSyncHint}</span></div>
        <div class="how"><Toggle checked={p.settings.auto_sync} label={t.autoSync} onchange={(on) => save({ auto_sync: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.deleteAfter}</b><span>{t.deleteAfterHint}</span></div>
        <div class="how"><Toggle checked={p.settings.delete_after_sync} label={t.deleteAfter} onchange={(on) => save({ delete_after_sync: on })} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.wifi}</b><span>{t.wifiHint}</span></div>
        <div class="how"><Toggle checked={p.settings.wifi} label={t.wifi} onchange={(on) => save({ wifi: on })} /></div>
      </div>
    </section>

    <h2 class="section-title">{t.stepDevice}</h2>
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.setClock}</b><span>{t.setClockHint}</span></div>
        <div class="how"><Toggle checked={p.settings.set_clock} label={t.setClock} onchange={(on) => save({ set_clock: on })} /></div>
      </div>
      {#if p.phase !== "unpaired" && p.phase !== "pairing"}
        <div class="setting">
          <div class="what"><b>{t.forget}</b><span>{t.forgetHint}</span></div>
          <div class="how"><ConfirmButton small label={t.forget} question={t.forgetConfirm} sure={t.forget} onconfirm={forget} /></div>
        </div>
      {/if}
    </section>
  {/if}
{/if}
{#if error}<p class="error-text note">{error}</p>{/if}

<style>
  .note {
    font-size: var(--fs-meta);
    margin: -4px 0 10px;
  }
  .what .state {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .panel {
    margin-bottom: 22px;
  }
</style>
