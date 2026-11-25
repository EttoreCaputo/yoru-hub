<!--
  The board on the desk, as a page of Settings: its firmware (and updating
  it), its WiFi, and the settings it keeps itself: first what it hears and how
  it sounds (the wake word, volume, chimes), then how it behaves; how it's
  reached, and its log, are advanced. Only the hardware is here: Yoru's voice and notch have
  their own pages. With several boards, first the list of them
  (name, how it's reached, firmware, the main one): the rest of the page is
  about the one picked there.
-->
<script lang="ts">
  import { onWifi } from "../port";
  import { call, why } from "../api";
  import { BUNDLED_VERSION } from "../bundled";
  import { i18n } from "../i18n/index.svelte";
  import { look } from "../look.svelte";
  import { store } from "../store.svelte";
  import type { BoardVoice, DeviceInfo, PortInfo, Transport, WifiInfo, WifiNetwork, YoruSetting, YoruSettings } from "../types";
  import { boldParts } from "../updates";
  import Toggle from "./Toggle.svelte";

  const t = $derived(i18n.t.board);
  const tb = $derived(i18n.t.boards);
  const several = $derived(store.boards.length > 1);
  const boardDot = (link: string) => (link === "connected" ? "done" : link === "searching" ? "think" : "wrong");
  const via = (b: DeviceInfo) => (onWifi(b.port) ? i18n.t.link.wifi : b.port ? i18n.t.link.usb : "");
  const where = (b: DeviceInfo) => (onWifi(b.port) ? (b.id ?? "") : (b.port ?? "")).replace("/dev/cu.", "");
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
    void store.boardId;
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
  /** The settings about hearing and sound, shown with the wake word. */
  const HEARING = ["wake_on", "wake_sound", "volume", "yoru_sounds", "mic_muted"];
  const hearing = $derived(HEARING.flatMap((n) => rows.filter(([name]) => name === n)));
  const behaviour = $derived(rows.filter(([name]) => !HEARING.includes(name)));

  // The wake word, which the settings table doesn't give.
  let voice = $state<BoardVoice | null>(null);
  $effect(() => {
    if (!live) return;
    void store.boardId;
    call<BoardVoice>("board_voice", { board: null }).then((v) => (voice = v), () => {});
  });
  async function setWakeWord(word: string) {
    const before = voice?.wake_word ?? null;
    if (voice) voice.wake_word = word;
    try {
      await call("set_board_wake_word", { board: null, word });
    } catch (e) {
      if (voice) voice.wake_word = before;
      tableError = why(e);
    }
  }
  let testing = $state(false);
  async function testVoice() {
    testing = true;
    tableError = "";
    try {
      await call("test_voice", { board: null, language: i18n.lang });
    } catch (e) {
      tableError = why(e);
    } finally {
      testing = false;
    }
  }
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
  /** After an answer the board keeps listening a few seconds (a host setting; never the notch). */
  let followUp = $state(false);
  $effect(() => {
    void call<{ follow_pointer: boolean; board_follow_up: boolean }>("status").then((s) => {
      followPointer = s?.follow_pointer ?? true;
      followUp = s?.board_follow_up ?? false;
    }, () => {});
  });
  const running = $derived(store.firmware?.state === "version" ? store.firmware.version : null);
  // Updated over its link instead of esptool (core's ota.rs).
  const overWifi = $derived(onWifi(store.port));
</script>

<div>
  {#if several}
    <h2 class="section-title">{tb.title}</h2>
    <p class="dim hint">{tb.hint}</p>
    <div class="cards">
      {#each store.boards as b (b.target)}
        <section class="panel card" class:on={b.current}>
          <div class="head">
            <span class="dot {boardDot(b.link)}"></span>
            <input
              type="text"
              class="bare bname"
              value={b.name ?? ""}
              placeholder={store.boardName({ ...b, name: null })}
              maxlength="24"
              aria-label={tb.name}
              disabled={!b.id}
              onchange={(e) => store.renameBoard(b, e.currentTarget.value)}
              onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            />
            {#if b.current}<span class="tag">{tb.shown}</span>{:else}<button class="btn small" onclick={() => store.showBoard(b)}>{tb.showThis}</button>{/if}
          </div>
          <div class="facts">
            {#if b.link === "searching"}<span>{tb.joining}</span>{:else if via(b)}<span>{tb.via(via(b), where(b))}</span>{/if}
            {#if b.firmware?.state === "version"}<span>{tb.firmware(b.firmware.version)}</span>{/if}
          </div>
          <div class="main">
            {#if b.primary}
              <button class="btn link star on" title={tb.primary} onclick={() => store.setPrimary(null)}>★ {tb.main}</button>
            {:else}
              <button class="btn link star" title={tb.primary} disabled={!b.id} onclick={() => store.setPrimary(b)}>☆ {tb.makeMain}</button>
            {/if}
          </div>
        </section>
      {/each}
    </div>
    {#if !store.boards.some((b) => b.primary)}<p class="faint hint">{tb.noMain}</p>{/if}
    <h2 class="of">{tb.settingsOf(store.boardName(store.shown))}</h2>
  {/if}
  {#if !live}
    <p class="dim nob">{store.permissionDenied ? t.permission : t.noBoard}</p>
    {#if store.message}<p class="mono faint">{store.message}</p>{/if}
  {/if}

  <h2 class="section-title" class:sub={several}>{t.firmware}</h2>
  <section class="panel">
    <div class="setting">
      <div class="what">
        <b>{running ? t.runs(running) : "—"}</b>
        <span>{BUNDLED_VERSION ? t.bundled(BUNDLED_VERSION) : ""}</span>
      </div>
      <div class="how">
        {#if store.job === "flash"}
          <span class="dim">{store.setup ? t.preparing : overWifi ? t.sending(Math.round(store.flashPercent)) : t.flashing(Math.round(store.flashPercent))}</span>
        {:else if store.update}
          <button class="btn primary" onclick={() => store.flash()}>{overWifi ? t.updateOverWifi(store.update.to) : t.updateTo(store.update.to)}</button>
        {:else}
          {#if live && running}<span class="tag">{t.upToDate}</span>{/if}
          <button class="btn quiet" disabled={store.job !== "idle"} onclick={() => store.flash()}>{overWifi ? t.sendOverWifi : t.flash}</button>
        {/if}
      </div>
    </div>
    {#if store.job === "flash"}
      <div class="meter flash" style:--p={store.flashPercent / 100} style:--c="var(--think)"><span></span></div>
    {/if}
    {#if overWifi && store.job !== "idle"}<p class="dim">{t.overWifi}</p>{/if}
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

  {#if live}
    <h2 class="section-title">{t.hearing}</h2>
    <section class="panel">
      <p class="sub">{t.settingsHint}</p>
      {#if !table && !tableError}<p class="dim">{t.reading}</p>{/if}
      {#if voice?.wake_word && voice.wake_words.length}
        <div class="setting">
          <div class="what"><b>{t.wakeWordLabel}</b></div>
          <div class="how">
            <select value={voice.wake_word} onchange={(e) => setWakeWord(e.currentTarget.value)} aria-label={t.wakeWordLabel}>
              {#each voice.wake_words as w (w)}<option value={w}>{t.wakeWords[w] ?? w}</option>{/each}
            </select>
          </div>
        </div>
      {/if}
      {#each hearing as [name, s] (name)}
        {@render row(name, s)}
      {/each}
    </section>

    <h2 class="section-title">{t.behaviour}</h2>
    <section class="panel">
      {#each behaviour as [name, s] (name)}
        {@render row(name, s)}
      {/each}
      <div class="setting">
        <div class="what"><b>{t.followPointer}</b></div>
        <div class="how"><Toggle checked={followPointer} label={t.followPointer} onchange={(on) => ((followPointer = on), call("set_follow_pointer", { on }))} /></div>
      </div>
      <div class="setting">
        <div class="what"><b>{t.followUp}</b><span>{t.followUpHint}</span></div>
        <div class="how"><Toggle checked={followUp} label={t.followUp} onchange={(on) => ((followUp = on), call("set_board_follow_up", { on }))} /></div>
      </div>
      {#if tableError}<p class="error-text">{tableError}</p>{/if}
      <div class="row foot"><button class="btn quiet small" onclick={() => call("yoru_show_menu", { board: null, open: true, page: null }).catch(() => {})}>{t.openMenu}</button></div>
    </section>
  {/if}

  {#if look.advanced}
    <h2 class="section-title">{i18n.t.settings.advanced}</h2>
    <section class="panel">
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
    <h3 class="section-title">{t.logs}</h3>
    <pre class="well mono log">{store.logs.length ? store.logs.slice(-120).join("\n") : t.noLogs}</pre>
  {/if}
</div>

{#snippet row(name: string, s: YoruSetting)}
  <div class="setting">
    <div class="what"><b>{t.names[name] ?? name.replaceAll("_", " ")}</b></div>
    <div class="how">
      {#if s.min === 0 && s.max === 1}
        <Toggle checked={s.value === 1} label={t.names[name] ?? name} onchange={(on) => setSetting(name, on ? 1 : 0)} />
      {:else}
        <input type="range" min={s.min} max={s.max} value={s.value} onchange={(e) => setSetting(name, Number(e.currentTarget.value))} aria-label={t.names[name] ?? name} />
        <span class="mono val">{s.value}</span>
      {/if}
      {#if name === "volume"}<button class="btn quiet small" disabled={testing} onclick={testVoice}>{t.test}</button>{/if}
    </div>
  </div>
{/snippet}

<style>
  .hint {
    margin: -4px 0 12px;
    font-size: var(--fs-meta);
    max-width: 620px;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 12px;
  }
  .card {
    display: grid;
    gap: 8px;
    padding: 14px 16px;
  }
  .panel.card + .panel.card {
    margin-top: 0;
  }
  .card.on {
    border-color: var(--pearl-deep);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  input.bname {
    flex: 1;
    min-width: 0;
    font: 600 var(--fs-head)/1.2 var(--sans);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--haze);
    font-size: var(--fs-meta);
  }
  .star {
    font-size: var(--fs-meta);
  }
  .star:hover:not(:disabled) {
    color: var(--pearl);
  }
  .star.on {
    color: var(--wait);
  }
  h2.of {
    margin: 36px 0 0;
    padding-top: 22px;
    border-top: 1px solid var(--rim);
    font: 650 var(--fs-head)/1.2 var(--display);
  }
  .section-title.sub {
    margin-top: 16px;
  }
  .nob {
    margin-bottom: 6px;
  }
  .sub {
    margin-bottom: 4px;
  }
  .flash {
    margin: 4px 0 10px;
  }
  .news {
    margin-top: 8px;
    font-size: var(--fs-body);
  }
  .news summary {
    cursor: pointer;
    color: var(--haze);
  }
  .news h3 {
    margin: 12px 0 4px;
    font-size: var(--fs-meta);
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
  pre.log {
    margin: 0;
    padding: 14px 16px;
    max-height: 280px;
    overflow: auto;
    color: var(--haze);
    white-space: pre-wrap;
    user-select: text;
  }
</style>
