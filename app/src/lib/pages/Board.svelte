<!--
  The board on the desk: how it's reached, its firmware (and updating it), its
  WiFi, its voice, and the settings it keeps itself.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { BUNDLED_VERSION } from "../bundled";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { PortInfo, Transport, WifiInfo, WifiNetwork, YoruSetting, YoruSettings } from "../types";
  import Toggle from "../ui/Toggle.svelte";
  import { boldParts } from "../updates";
  import Voice from "../ui/Voice.svelte";

  const t = $derived(i18n.t.board);
  const live = $derived(store.link === "connected");

  // Connection.
  let ports = $state<PortInfo[]>([]);
  let transport = $state<Transport>("auto");
  let portSetting = $state<string | null>(null);
  $effect(() => {
    void call<PortInfo[]>("list_ports").then((p) => (ports = p ?? []), () => {});
    void call<{ port_setting: string | null }>("status").then((s) => (portSetting = s?.port_setting ?? null), () => {});
  });
  const setTransport = async (v: Transport) => ((transport = v), await call("set_transport", { transport: v }));
  const setPort = async (v: string) => ((portSetting = v || null), await call("set_port", { port: v || null }));

  // WiFi.
  let wifi = $state<WifiInfo | null>(null);
  let networks = $state<WifiNetwork[] | null>(null);
  let scanning = $state(false);
  let ssid = $state("");
  let password = $state("");
  let wifiError = $state("");
  let wifiBusy = $state(false);
  async function readWifi() {
    try {
      wifi = await call<WifiInfo>("wifi_info", { board: null });
      if (wifi) transport = wifi.transport;
    } catch (e) {
      wifiError = why(e);
    }
  }
  $effect(() => {
    if (live) void readWifi();
  });
  async function scan() {
    scanning = true;
    wifiError = "";
    try {
      networks = (await call<{ networks: WifiNetwork[] }>("wifi_scan", { board: null })).networks;
    } catch (e) {
      wifiError = why(e);
    } finally {
      scanning = false;
    }
  }
  async function wifiDo(cmd: string, args: Record<string, unknown> = {}) {
    wifiBusy = true;
    wifiError = "";
    try {
      await call(cmd, { board: null, ...args });
      password = "";
      networks = null;
      await readWifi();
    } catch (e) {
      wifiError = why(e);
    } finally {
      wifiBusy = false;
    }
  }

  // The board's own settings.
  let table = $state<YoruSettings | null>(null);
  let tableError = $state("");
  $effect(() => {
    if (!live) return;
    void store.boardId;
    table = null;
    tableError = "";
    call<YoruSettings>("yoru_settings", { board: null }).then(
      (s) => (table = s),
      (e) => (tableError = why(e)),
    );
  });
  const rows = $derived(table ? Object.entries(table).filter((e): e is [string, YoruSetting] => typeof e[1] === "object" && e[1] !== null && "value" in e[1]) : []);
  const wakeWord = $derived(typeof table?.wake_word === "string" ? table.wake_word : null);
  async function setSetting(name: string, value: number) {
    const s = table?.[name];
    if (typeof s !== "object") return;
    const before = s.value;
    s.value = value;
    try {
      await call("yoru_set_setting", { board: null, name, value });
    } catch (e) {
      s.value = before;
      tableError = why(e);
    }
  }

  let followPointer = $state(true);
  $effect(() => {
    void call<{ follow_pointer: boolean }>("status").then((s) => (followPointer = s?.follow_pointer ?? true), () => {});
  });
  const running = $derived(store.firmware?.state === "version" ? store.firmware.version : null);
</script>

<div class="page">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  <h2 class="section-title">{t.connection}</h2>
  <section class="panel">
    {#if !live}
      <p class="dim nob">{store.permissionDenied ? t.permission : t.noBoard}</p>
      {#if store.message}<p class="mono faint">{store.message}</p>{/if}
    {/if}
    <div class="setting">
      <div class="what"><b>{t.transport}</b>{#if live}<span class="mono">{store.port}</span>{/if}</div>
      <div class="how">
        <select value={transport} onchange={(e) => setTransport(e.currentTarget.value as Transport)} aria-label={t.transport}>
          {#each ["auto", "usb", "wifi"] as const as v (v)}<option value={v}>{t.transports[v]}</option>{/each}
        </select>
      </div>
    </div>
    <div class="setting">
      <div class="what"><b>{t.port}</b></div>
      <div class="how">
        <select value={portSetting ?? ""} onchange={(e) => setPort(e.currentTarget.value)} aria-label={t.port}>
          <option value="">{t.autoPort}</option>
          {#each ports as p (p.name)}<option value={p.name}>{p.name}{p.product ? ` · ${p.product}` : ""}</option>{/each}
        </select>
      </div>
    </div>
  </section>

  <h2 class="section-title">{t.firmware}</h2>
  <section class="panel">
    <div class="setting">
      <div class="what">
        <b>{running ? t.runs(running) : "—"}</b>
        <span>{BUNDLED_VERSION ? t.bundled(BUNDLED_VERSION) : ""}</span>
      </div>
      <div class="how">
        {#if store.job === "flash"}
          <span class="dim">{store.setup ? t.preparing : t.flashing(Math.round(store.flashPercent))}</span>
        {:else if store.update}
          <button class="btn primary" onclick={() => store.flash()}>{t.updateTo(store.update.to)}</button>
        {:else}
          {#if live && running}<span class="tag">{t.upToDate}</span>{/if}
          <button class="btn quiet" disabled={store.job !== "idle"} onclick={() => store.flash()}>{t.flash}</button>
        {/if}
      </div>
    </div>
    {#if store.job === "flash"}
      <div class="bar"><span style:width="{store.flashPercent}%"></span></div>
    {/if}
    {#if store.flashedAt && store.job === "idle"}<p class="dim">{t.flashed}</p>{/if}
    {#if store.flashError}<p class="error-text">{store.flashError}</p>{/if}
    {#if store.update?.changes.length}
      <details class="news">
        <summary>{t.whatsNew}</summary>
        {#each store.update.changes as r (r.version)}
          <h3 class="mono">{r.version}</h3>
          <ul>
            {#each r.notes as n, i (i)}<li>{#each boldParts(n) as p, j (j)}{#if p.bold}<b>{p.text}</b>{:else}{p.text}{/if}{/each}</li>{/each}
          </ul>
        {/each}
      </details>
    {/if}
  </section>

  {#if live}
    <h2 class="section-title">{t.wifi}</h2>
    <section class="panel">
      <div class="setting">
        <div class="what">
          <b>{wifi?.board ? (wifi.board.enabled ? (wifi.board.ssid ? t.wifiOn(wifi.board.ssid) : t.wifiNone) : t.wifiOff) : "—"}</b>
          {#if wifi?.board?.ip}<span class="mono">{wifi.board.ip}</span>{/if}
        </div>
        <div class="how">
          {#if wifi?.board?.ssid}
            <button class="btn quiet small" disabled={wifiBusy} onclick={() => wifiDo("wifi_enable", { on: !wifi?.board?.enabled })}>{wifi.board.enabled ? t.turnOff : t.turnOn}</button>
            <button class="btn quiet small" disabled={wifiBusy} onclick={() => wifiDo("wifi_forget")}>{t.forget}</button>
            {#if !wifi.paired.some((p) => p.id === store.boardId)}<button class="btn small" disabled={wifiBusy} onclick={() => wifiDo("wifi_pair")}>{t.pair}</button>{/if}
          {/if}
          <button class="btn small" disabled={scanning} onclick={scan}>{scanning ? t.scanning : t.scan}</button>
        </div>
      </div>
      {#if networks}
        <form class="join" onsubmit={(e) => (e.preventDefault(), wifiDo("wifi_setup", { ssid, password }))}>
          <select bind:value={ssid} aria-label="SSID">
            <option value="" disabled>SSID</option>
            {#each networks as n (n.ssid)}<option value={n.ssid}>{n.ssid} · {n.rssi} dBm{n.secure ? " · 🔒" : ""}</option>{/each}
          </select>
          <input type="password" bind:value={password} placeholder={t.password} autocomplete="off" />
          <button class="btn primary" type="submit" disabled={!ssid || wifiBusy}>{t.join}</button>
        </form>
      {/if}
      {#if wifiError || wifi?.error}<p class="error-text">{wifiError || wifi?.error}</p>{/if}
    </section>
  {/if}

  <h2 class="section-title">{t.voice}</h2>
  <Voice {wakeWord} />

  {#if live}
    <h2 class="section-title">{t.settings}</h2>
    <section class="panel">
      <p class="sub">{t.settingsHint}</p>
      {#if !table && !tableError}<p class="dim">{t.reading}</p>{/if}
      {#each rows as [name, s] (name)}
        <div class="setting">
          <div class="what"><b>{t.names[name] ?? name.replaceAll("_", " ")}</b></div>
          <div class="how">
            {#if s.min === 0 && s.max === 1}
              <Toggle checked={s.value === 1} label={t.names[name] ?? name} onchange={(on) => setSetting(name, on ? 1 : 0)} />
            {:else}
              <input type="range" min={s.min} max={s.max} value={s.value} onchange={(e) => setSetting(name, Number(e.currentTarget.value))} aria-label={t.names[name] ?? name} />
              <span class="mono val">{s.value}</span>
            {/if}
          </div>
        </div>
      {/each}
      <div class="setting">
        <div class="what"><b>{t.followPointer}</b></div>
        <div class="how"><Toggle checked={followPointer} label={t.followPointer} onchange={(on) => ((followPointer = on), call("set_follow_pointer", { on }))} /></div>
      </div>
      {#if tableError}<p class="error-text">{tableError}</p>{/if}
      <div class="row foot"><button class="btn quiet small" onclick={() => call("yoru_show_menu", { board: null, open: true, page: null }).catch(() => {})}>{t.openMenu}</button></div>
    </section>
  {/if}

  <details class="logs">
    <summary class="section-title">{t.logs}</summary>
    <pre class="mono">{store.logs.length ? store.logs.slice(-120).join("\n") : t.noLogs}</pre>
  </details>
</div>

<style>
  .nob {
    margin-bottom: 6px;
  }
  .sub {
    margin-bottom: 4px;
  }
  select {
    width: auto;
    min-width: 200px;
  }
  .bar {
    height: 4px;
    border-radius: 4px;
    background: var(--ink-3);
    overflow: hidden;
    margin: 4px 0 10px;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--think);
    transition: width 300ms;
  }
  .news {
    margin-top: 8px;
    font-size: 13px;
  }
  .news summary {
    cursor: pointer;
    color: var(--haze);
  }
  .news h3 {
    margin: 12px 0 4px;
    font-size: 12px;
    color: var(--pearl-mid);
  }
  .news ul {
    margin: 0;
    padding-left: 18px;
    color: var(--haze);
  }
  .join {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: 8px;
    padding: 4px 0 12px;
  }
  .join select {
    width: 100%;
  }
  input[type="range"] {
    width: 160px;
  }
  .val {
    width: 30px;
    text-align: right;
    color: var(--haze);
  }
  .foot {
    padding-top: 10px;
    border-top: 1px solid var(--rim);
  }
  .logs summary {
    cursor: pointer;
    list-style: none;
  }
  .logs summary::before {
    content: "▸ ";
  }
  .logs[open] summary::before {
    content: "▾ ";
  }
  .logs pre {
    margin: 0;
    padding: 14px 16px;
    max-height: 280px;
    overflow: auto;
    border-radius: var(--r);
    background: var(--ink);
    border: 1px solid var(--rim);
    color: var(--haze);
    white-space: pre-wrap;
    user-select: text;
  }
</style>
