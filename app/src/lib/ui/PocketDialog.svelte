<!--
  The Pocket recorder (docs/pocket.md), opened from its row in the sidebar,
  after the vendor's "device sync" panel: the device (connection, battery,
  storage), the recordings on it and which are already meetings, the new
  ones to bring in (all, or those picked), and the download under way.
  Unpaired, it walks the user through the reset that lets Yoru pair it.
  Its settings are in Settings, under Pocket.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import { tabs } from "../tabs.svelte";
  import type { PocketRecording, PocketStatus } from "../types";
  import { clock } from "./when";
  import ConfirmButton from "./ConfirmButton.svelte";
  import Dialog from "./Dialog.svelte";
  import Icon from "./Icon.svelte";

  let { pocket, onclose }: { pocket: PocketStatus; onclose: () => void } = $props();

  const t = $derived(i18n.t.pocket);
  let error = $state("");
  /** The new recordings picked to bring in. */
  let picked = $state<string[]>([]);
  let showAll = $state(false);

  const fresh = $derived(pocket.recordings.filter((r) => !r.meeting));
  const kept = $derived(pocket.recordings.filter((r) => r.meeting));
  const connected = $derived(pocket.phase === "connected");
  const busy = $derived(pocket.checking || !!pocket.transfer);
  const percent = (f: number) => Math.round(f * 100);
  const when = (r: PocketRecording) => (r.started_at ? `${i18n.date(r.started_at)} · ${i18n.time(r.started_at)}` : r.name);
  const gb = (mb: number) => (mb / 1024).toFixed(1);

  // Picks of recordings no longer new are dropped.
  $effect(() => {
    const names = new Set(fresh.map((r) => r.name));
    if (picked.some((n) => !names.has(n))) picked = picked.filter((n) => names.has(n));
  });

  async function act(command: string, args?: Record<string, unknown>) {
    error = "";
    try {
      await call(command, args);
    } catch (e) {
      error = why(e);
    }
  }

  const sync = () => act("pocket_sync");
  const bring = () => act("pocket_download", { names: picked.length ? picked : fresh.map((r) => r.name) }).then(() => (picked = []));
  const toggle = (name: string, on: boolean) => (picked = on ? [...picked, name] : picked.filter((n) => n !== name));

  function openMeeting(id: string) {
    onclose();
    tabs.open({ kind: "meeting", id });
  }

  const dot = $derived(
    connected ? "done" : pocket.phase === "refused" || pocket.phase === "no_bluetooth" ? "wrong" : pocket.phase === "unpaired" ? "" : "wait",
  );
</script>

<Dialog title={pocket.name ?? "Pocket"} {onclose}>
<div class="pocket">
  <div class="head">
    <span class="glyph"><Icon name="mic" size={20} stroke={1.6} /></span>
    <div class="who">
      <span class="dim small"><span class="dot {dot}"></span> {t.phases[pocket.phase]}{#if pocket.recording} · <span class="rec-now">{t.recordingNow}</span>{/if}</span>
    </div>
    {#if connected}
      <button class="btn small sync" disabled={busy} onclick={sync}>{pocket.checking ? t.checking : t.syncNow}</button>
    {/if}
  </div>
  {#if connected}
    <div class="facts small dim">
      {#if pocket.battery !== null}<span title={t.battery}><Icon name="battery" size={15} /> {pocket.battery}%</span>{/if}
      {#if pocket.free_mb !== null && pocket.total_mb}
        <span title={t.storage}><Icon name="hard-drive" size={15} /> {t.free(gb(pocket.free_mb), gb(pocket.total_mb))}</span>
      {/if}
      {#if pocket.firmware}<span class="faint">{t.firmware(pocket.firmware)}</span>{/if}
    </div>
  {/if}

  {#if pocket.phase === "unpaired" || pocket.phase === "refused"}
    <div class="setup">
      <p>{pocket.phase === "refused" ? t.refused : t.intro}</p>
      <ol>
        <li>{t.step1}</li>
        <li>{t.step2}</li>
        <li>{t.step3}</li>
      </ol>
      <button class="btn primary" onclick={() => act("pocket_pair", { on: true })}><Icon name="bluetooth" size={15} /> {t.pair}</button>
      <p class="faint small">{t.pairNote}</p>
    </div>
  {:else if pocket.phase === "pairing"}
    <div class="setup">
      <p>{t.pairing}</p>
      <button class="btn small" onclick={() => act("pocket_pair", { on: false })}>{t.cancel}</button>
    </div>
  {:else if pocket.phase === "no_bluetooth"}
    <p class="error-text">{pocket.error ?? t.noBluetooth}</p>
  {:else if connected || pocket.recordings.length}
    {#if pocket.transfer}
      {@const tr = pocket.transfer}
      <div class="transfer">
        <span class="small">{t.downloading(when(pocket.recordings.find((r) => r.name === tr.name) ?? { date: "", name: tr.name, seconds: 0, started_at: null, meeting: null }))}</span>
        <span class="meter"><span style:--p={tr.total ? tr.bytes / tr.total : 0}></span></span>
        <span class="faint small mono">{tr.total ? percent(tr.bytes / tr.total) : 0}%{#if tr.queued} · {t.queued(tr.queued)}{/if}</span>
      </div>
    {/if}

    {#if fresh.length}
      <div class="group">
        <div class="group-head">
          <b class="small">{t.newOnes(fresh.length)}</b>
          {#if connected && !busy}
            <button class="btn small quiet" onclick={() => (picked = picked.length === fresh.length ? [] : fresh.map((r) => r.name))}>
              {picked.length === fresh.length ? t.selectNone : t.selectAll}
            </button>
            <button class="btn small primary" onclick={bring}>{t.bring(picked.length || fresh.length)}</button>
          {/if}
        </div>
        {#each fresh as r (r.name)}
          <label class="row">
            <input type="checkbox" checked={picked.includes(r.name)} disabled={!connected || busy} onchange={(e) => toggle(r.name, e.currentTarget.checked)} />
            <span>{when(r)}</span>
            <span class="faint small mono">{clock(r.seconds)}</span>
            {#if pocket.transfer?.name === r.name}<span class="tag">{t.inProgress}</span>{/if}
          </label>
        {/each}
      </div>
    {:else if connected && !pocket.checking && pocket.synced_at}
      <p class="dim small upto"><span class="dot done"></span> {t.upToDate}</p>
    {/if}

    {#if kept.length}
      <div class="group">
        <div class="group-head">
          <b class="small">{t.kept(kept.length)}</b>
          <button class="btn small quiet" onclick={() => (showAll = !showAll)}>{showAll ? t.hide : t.show}</button>
        </div>
        {#if showAll}
          {#each kept as r (r.name)}
            <div class="row">
              <span>{when(r)}</span>
              <span class="faint small mono">{clock(r.seconds)}</span>
              <span class="end">
                {#if store.meetings.some((m) => m.id === r.meeting)}
                  <button class="btn small" onclick={() => openMeeting(r.meeting!)}>{t.open}</button>
                {/if}
                {#if connected}
                  <ConfirmButton small icon="trash-2" label={t.deleteFromPocket} question={t.deleteConfirm} sure={t.delete} disabled={busy} onconfirm={() => act("pocket_delete", { name: r.name })} />
                {/if}
              </span>
            </div>
          {/each}
        {/if}
      </div>
    {/if}
  {:else}
    <p class="dim small">{t.nearHint}</p>
  {/if}

  {#if pocket.error && pocket.phase !== "no_bluetooth" && pocket.phase !== "refused"}<p class="error-text small">{pocket.error}</p>{/if}
  {#if error}<p class="error-text small">{error}</p>{/if}
</div>
</Dialog>

<style>
  .pocket {
    width: min(480px, 80vw);
  }
  .small {
    font-size: var(--fs-meta);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .glyph {
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    border-radius: 10px;
    background: var(--ink-3);
    color: var(--pearl-mid);
  }
  .who {
    display: grid;
    gap: 2px;
  }
  .who .dot {
    display: inline-block;
    vertical-align: middle;
    margin-right: 2px;
  }
  .rec-now {
    color: var(--wrong);
  }
  .sync {
    margin-left: auto;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 16px;
    margin: 10px 0 0 48px;
  }
  .facts > span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }
  .setup {
    margin-top: 14px;
  }
  .setup ol {
    margin: 8px 0 14px;
    padding-left: 20px;
    display: grid;
    gap: 6px;
    color: var(--haze);
  }
  .setup .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .setup .faint {
    margin-top: 8px;
  }
  .transfer {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 6px 12px;
    margin-top: 14px;
  }
  .transfer .meter {
    grid-column: 1 / -1;
    grid-row: 2;
  }
  .group {
    margin-top: 14px;
    padding-top: 12px;
    border-top: 1px solid var(--rim);
  }
  .group-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .group-head b {
    margin-right: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 32px;
  }
  .row .end {
    display: flex;
    gap: 8px;
    margin-left: auto;
  }
  .upto {
    margin-top: 12px;
  }
  .upto .dot {
    display: inline-block;
    vertical-align: middle;
  }
</style>
