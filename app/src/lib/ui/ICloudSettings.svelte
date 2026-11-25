<!--
  Settings > iCloud (docs/icloud.md): sync between this user's Macs on or
  off, meetings with or without audio, and what the engine is doing: the
  account, this Mac, which Mac fires timers and routines (the doer), the
  last records sent and received, errors.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ICloud, ICloudStatus } from "../types";
  import Toggle from "./Toggle.svelte";
  import { ago } from "./when";

  const t = $derived(i18n.t.icloud);
  let cloud = $state<ICloud | null>(null);
  let error = $state("");

  async function read() {
    cloud = await call<ICloud>("icloud_status").catch(() => null);
  }
  $effect(() => {
    void read();
  });
  // The engine's own news, as it comes.
  const s = $derived<Partial<ICloudStatus>>(store.icloud ?? cloud?.status ?? {});

  async function set(cmd: "set_icloud" | "set_icloud_audio", on: boolean) {
    error = "";
    try {
      await call(cmd, { on });
    } catch (e) {
      error = why(e);
    }
    await read();
  }

  const account = $derived(s.account ? (t.accounts[s.account] ?? s.account) : t.accounts.unknown);
  const failing = $derived(!!s.errors?.length && (s.last_error ?? 0) > Math.max(s.last_sent ?? 0, s.last_fetched ?? 0));
  const dot = $derived(!cloud?.on ? "" : failing ? "wrong" : s.account === "available" ? "done" : "wait");
  const when = (at: number | undefined) => (at ? ago(at, store.now) : t.never);
</script>

{#if !cloud}
  <p class="dim">{t.loading}</p>
{:else}
  <section class="panel">
    <div class="setting">
      <div class="what">
        <b>{t.sync}</b>
        <span>{s.entitled === false ? t.notEntitled : t.syncHint}</span>
      </div>
      <div class="how"><Toggle checked={cloud.on} disabled={s.entitled === false} label={t.sync} onchange={(on) => set("set_icloud", on)} /></div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.audio}</b><span>{t.audioHint}</span></div>
      <div class="how"><Toggle checked={cloud.audio} label={t.audio} onchange={(on) => set("set_icloud_audio", on)} /></div>
    </div>
  </section>

  {#if cloud.on}
    <h2 class="section-title">{t.now}</h2>
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.account}</b></div>
        <div class="how state"><span class="dot {dot}"></span>{account}</div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.doer}</b><span>{t.doerHint}</span></div>
        <div class="how">{s.is_doer ? t.thisMac : s.doer ? t.otherMac(s.doer) : t.nobody}</div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.sent}</b></div>
        <div class="how">{when(s.last_sent)}{s.sent ? ` · ${t.records(s.sent)}` : ""}</div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.received}</b></div>
        <div class="how">{when(s.last_fetched)}{s.received ? ` · ${t.records(s.received)}` : ""}</div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.device}</b><span>{t.deviceHint}</span></div>
        <div class="how mono">{s.device ?? ""}</div>
      </div>
    </section>
    {#if s.errors?.length}
      <h2 class="section-title">{t.errors}</h2>
      <section class="panel errors">
        {#each s.errors.slice(-5).reverse() as e, i (i)}<p class="error-text">{e}</p>{/each}
      </section>
    {/if}
  {/if}
{/if}
{#if error}<p class="error-text note">{error}</p>{/if}

<style>
  .note {
    font-size: var(--fs-meta);
    margin: -4px 0 10px;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--fs-meta);
  }
  .panel {
    margin-bottom: 22px;
  }
  .errors {
    padding: 10px 14px;
  }
  .errors p {
    margin: 4px 0;
    font-size: var(--fs-meta);
    overflow-wrap: anywhere;
  }
</style>
