<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { monitor, type BoardKind, type FirmwareInfo, type PortInfo, type Transport, type WifiInfo } from "../monitor.svelte";
  import { boldParts } from "../updates";
  import Advanced from "./Advanced.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { onboarding } from "../onboarding/state.svelte";
  import { settingsNav } from "./nav.svelte";
  import Section from "./Section.svelte";
  import WifiSetup from "./WifiSetup.svelte";
  import YoruSettings from "./YoruSettings.svelte";

  const td = $derived(i18n.t.settings.device);
  const tc = $derived(i18n.t.settings.common);
  const tg = $derived(i18n.t.settings.general);

  let ports = $state<PortInfo[]>([]);
  $effect(() => {
    const load = () => monitor.listPorts().then((p) => (ports = p));
    load();
    const id = setInterval(load, 2000);
    return () => clearInterval(id);
  });

  // The connection: USB, WiFi or both. The board's WiFi reads itself, in WifiSetup.
  const TRANSPORTS: Transport[] = ["auto", "usb", "wifi"];
  let wifi = $state<WifiInfo | null>(null);
  let transportBusy = $state(false);
  let transportError = $state("");
  const transport = $derived(wifi?.transport ?? "auto");
  const onWifi = $derived(monitor.link === "connected" && !!monitor.port?.startsWith("WiFi"));

  async function setTransport(key: Transport) {
    transportBusy = true;
    transportError = "";
    try {
      await monitor.setTransport(key);
      wifi = await monitor.wifiInfo();
    } catch (e) {
      transportError = String(e);
    } finally {
      transportBusy = false;
    }
  }

  // The firmware: esptool flashes the one bundled with the app.
  let firmware = $state<FirmwareInfo | null>(null);
  let confirming = $state(false);
  $effect(() => {
    let alive = true;
    monitor.firmwareInfo().then((f) => alive && (firmware = f));
    return () => {
      alive = false;
    };
  });
  // The port esptool will use: the pinned one, else the board shown's (pick it
  // in the title bar, a blank one too), else the only Espressif device plugged in.
  const boards = $derived(ports.filter((p) => p.is_board));
  const shownPort = $derived.by(() => {
    const t = monitor.boards.find((b) => b.current)?.target;
    return t && !t.startsWith("wifi:") ? t : null;
  });
  const onWifiOnly = $derived(!shownPort && !!monitor.boards.find((b) => b.current)?.target.startsWith("wifi:"));
  const target = $derived(monitor.portSetting ?? shownPort ?? (boards.length === 1 && !onWifiOnly ? boards[0].name : null));
  // What runs on each port, for the list of ports.
  const onPort = (name: string) => {
    const b = monitor.boards.find((d) => d.target === name);
    return b?.kind ? td.kinds[b.kind] : "ESP32-S3";
  };
  const busy = $derived(monitor.job !== "idle");
  const onBoard = $derived.by(() => {
    const fw = monitor.boardFirmware;
    if (monitor.link !== "connected") return { text: "—", note: tc.offline };
    if (!fw) return { text: "—", note: td.asking2 };
    if (fw.state === "version") return { text: `v${fw.version}`, note: fw.idf ? `IDF ${fw.idf}` : "" };
    if (fw.state === "legacy") return { text: td.before020, note: td.unversioned };
    return { text: td.none, note: td.blank };
  });
  async function identify() {
    await monitor.identify(target);
    firmware = await monitor.firmwareInfo();
  }
  async function flash() {
    confirming = false;
    await monitor.flash(target);
    firmware = await monitor.firmwareInfo();
  }

  const kinds: BoardKind[] = ["dualeye", "yoru"];
  // The image a flash would write, for its size.
  const targetImage = $derived(firmware?.bundled.find((b) => b.kind === monitor.targetKind) ?? null);

  const hex = (n: number) => n.toString(16).padStart(4, "0");
  const kb = (n: number) => `${i18n.number(Math.round(n / 1024))} KB`;
</script>

<Section anchor="general" title={tg.title}>
  <div class="actions">
    <button class="btn" onclick={() => ((settingsNav.open = false), onboarding.redo())}>{tg.redo}</button>
    <InfoTip text={tg.redoTip} />
    <button class="btn" onclick={() => ((settingsNav.open = false), onboarding.startTour())}>{tg.tour}</button>
    <InfoTip text={tg.tourTip} />
  </div>
</Section>

{#if monitor.boards.length > 1}
  <Section anchor="boards" title={td.boards} tip={td.boardsTip}>
    <p class="hint">{td.primary}</p>
    <div class="select">
      <select value={monitor.primary ?? ""} aria-label={td.primary} onchange={(e) => monitor.setPrimary(e.currentTarget.value || null)}>
        <option value="">{td.noPrimary}</option>
        {#each monitor.boards.filter((b) => b.id) as b (b.target)}
          <option value={b.id}>{b.kind ? td.kinds[b.kind] : b.target} · {b.id}</option>
        {/each}
      </select>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 6.5 8 10l3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
    </div>
  </Section>
{/if}

{#if monitor.boardKind === "yoru"}
  <Section anchor="yoru" title={td.yoru} tip={td.yoruTip}>
    <YoruSettings />
  </Section>
{/if}

<Section anchor="connection" title={td.connection} tip={td.connectionTip}>
  <div class="ports">
    {#each TRANSPORTS as key (key)}
      {@const [name, note] = td.transports[key]}
      <label class="port" class:checked={transport === key}>
        <input type="radio" name="transport" checked={transport === key} disabled={transportBusy} onchange={() => setTransport(key)} />
        <span class="radio"></span>
        <span class="pname">{name}</span>
        <span class="pmeta">{note}</span>
      </label>
    {/each}
  </div>
  {#if monitor.link === "connected" && monitor.port}
    <p class="hint conn">{onWifi ? td.connectedWifi(monitor.port) : td.connectedUsb(monitor.port)}</p>
  {/if}
  {#if transport !== "usb" && wifi && !wifi.paired.length}
    <p class="hint">{td.noPaired}</p>
  {/if}
  {#if transportError}<p class="hint error">{transportError}</p>{/if}
</Section>

{#if monitor.message && monitor.link !== "connected"}
  <section class="alert" class:error={monitor.link === "offline"}>
    <h3>{monitor.link === "offline" ? td.lost : td.waiting}</h3>
    <p class="mono">{monitor.message}</p>
    {#if monitor.permissionDenied}
      <p class="hint">{td.linux[0]}<code>dialout</code>{td.linux[1]}</p>
      <pre>sudo usermod -aG dialout "$USER"</pre>
    {/if}
  </section>
{/if}

<Section class="wifi" anchor="wifi" title={td.wifi} tip={td.setupTip}>
  <WifiSetup bind:wifi />
</Section>

<Section anchor="firmware" title={td.firmware}>
  {#if !firmware}
    <p class="empty">{td.checking}</p>
  {:else}
    <dl class="facts">
      <div><dt>{td.onBoard}</dt><dd>{onBoard.text} {#if onBoard.note}<span class="muted sub">{onBoard.note}</span>{/if}</dd></div>
      <div>
        <dt>{td.boardKind}</dt>
        <dd>
          {#if monitor.boardKind}
            {td.kinds[monitor.boardKind]}
          {:else}
            <select bind:value={monitor.flashKind} disabled={busy}>
              <option value={null}>—</option>
              {#each kinds as kind (kind)}<option value={kind}>{td.kinds[kind]}</option>{/each}
            </select>
          {/if}
        </dd>
      </div>
      <div>
        <dt>{td.inApp}</dt>
        <dd>
          {monitor.bundled ? `v${monitor.bundled.version}` : "—"}
          {#if monitor.update}<span class="muted sub new">{td.newer}</span>{:else if monitor.bundled}<span class="muted sub">IDF {monitor.bundled.idf}</span>{/if}
        </dd>
      </div>
    </dl>
    {#if !monitor.boardKind}<p class="hint">{td.pickKind}</p>{/if}
    {#if monitor.update && monitor.update.changes.length && monitor.job !== "flash"}
      <div class="changes">
        <p class="hint">{td.brings(monitor.update.to)}</p>
        {#each monitor.update.changes as release (release.version)}
          <div class="release">
            <span class="ver">v{release.version}</span>
            <ul>
              {#each release.notes as line, i (i)}<li>{#each boldParts(line) as p, j (j)}{#if p.bold}<b>{p.text}</b>{:else}{p.text}{/if}{/each}</li>{/each}
            </ul>
          </div>
        {/each}
      </div>
    {/if}
    {#if monitor.setup}
      <div class="progress" class:indeterminate={monitor.setup.percent === null}>
        <span style:width="{monitor.setup.percent ?? 30}%"></span>
      </div>
      <p class="hint setup">{td.preparing(monitor.setup.message)}{monitor.setup.percent !== null ? ` ${Math.round(monitor.setup.percent)}%` : ""}</p>
    {:else if monitor.job === "flash"}
      <div class="progress" role="progressbar" aria-valuenow={Math.round(monitor.flashPercent)} aria-valuemin={0} aria-valuemax={100}>
        <span style:width="{monitor.flashPercent}%"></span>
      </div>
      <p class="hint">{td.writing(Math.round(monitor.flashPercent))}</p>
    {:else if confirming}
      <p class="hint">{td.confirm(target ?? "")}</p>
      <div class="actions">
        <button class="btn primary" onclick={flash}>{td.flashNow}</button>
        <button class="btn" onclick={() => (confirming = false)}>{tc.cancel}</button>
      </div>
    {:else}
      <div class="actions">
        <button class="btn primary" disabled={busy || !target || !monitor.targetKind} onclick={() => (confirming = true)}>{monitor.update ? td.updateTo(monitor.update.to) : td.flashFirmware}</button>
      </div>
      {#if !target}<p class="hint">{onWifiOnly ? td.flashNeedsUsb : boards.length > 1 ? td.several : td.noBoard}</p>{/if}
    {/if}
  {/if}
  {#if monitor.jobError}
    <section class="alert error">
      <h3>{td.failed}</h3>
      <p class="mono">{monitor.jobError}</p>
      {#if /busy|Errno 16/i.test(monitor.jobError)}
        <p class="hint">{td.busyPort[0]}<code>idf.py monitor</code>{td.busyPort[1]}<code>dualeye</code>{td.busyPort[2]}</p>
      {/if}
    </section>
  {:else if monitor.flashedAt && monitor.job === "idle"}
    <p class="ok">{td.flashed}</p>
  {/if}
  {#if monitor.jobLog.length}
    <details open={busy || !!monitor.jobError}>
      <summary>{td.output}</summary>
      <div class="console joblog">
        {#each monitor.jobLog as line, i (i)}<div class="log">{line}</div>{/each}
      </div>
    </details>
  {/if}
</Section>

<Advanced tab="device">
  {#if transport !== "wifi"}
    <Section anchor="serial" title={td.serial} tip={td.serialTip}>
      <div class="ports">
        <label class="port" class:checked={monitor.portSetting === null}>
          <input type="radio" name="port" checked={monitor.portSetting === null} onchange={() => monitor.setPort(null)} />
          <span class="radio"></span>
          <span class="pname">{td.automatic}</span>
          <span class="pmeta">{monitor.portSetting === null && monitor.port ? monitor.port : "303a:*"}</span>
        </label>
        {#each ports as p (p.name)}
          <label class="port" class:checked={monitor.portSetting === p.name}>
            <input type="radio" name="port" checked={monitor.portSetting === p.name} onchange={() => monitor.setPort(p.name)} />
            <span class="radio"></span>
            <span class="pname">{p.name}</span>
            <span class="pmeta">{hex(p.vid)}:{hex(p.pid)}{p.is_board ? ` · ${onPort(p.name)}` : ""}</span>
          </label>
        {:else}
          <p class="empty">{td.noPorts}</p>
        {/each}
      </div>
    </Section>
  {/if}
  <Section anchor="board" title={td.board} tip={td.boardTip}>
    {#if target}
      <div class="target">
        <span class="chipdot" class:found={monitor.chip?.port === target}></span>
        <span class="pname">{target}</span>
        <button class="btn" disabled={busy || !firmware} onclick={identify}>{monitor.job === "identify" ? td.reading : td.identify}</button>
      </div>
    {:else}
      <p class="empty">{onWifiOnly ? td.flashNeedsUsb : boards.length > 1 ? td.several : td.noBoard}</p>
    {/if}
    {#if monitor.chip && monitor.chip.port === target}
      <dl class="facts chipinfo">
        <div class="wide"><dt>{td.chip}</dt><dd>{monitor.chip.chip ?? "—"}</dd></div>
        <div><dt>{td.flash}</dt><dd>{monitor.chip.flash_size ?? "—"}</dd></div>
        <div><dt>{td.crystal}</dt><dd>{monitor.chip.crystal ?? "—"}</dd></div>
        <div class="wide"><dt>MAC</dt><dd>{monitor.chip.mac ?? "—"}</dd></div>
        {#if monitor.chip.features}<div class="wide"><dt>{td.features}</dt><dd class="small">{monitor.chip.features}</dd></div>{/if}
      </dl>
    {/if}
    {#if firmware}
      <dl class="facts chipinfo">
        <div><dt>{td.image}</dt><dd>merged-binary.bin</dd></div>
        {#if targetImage}<div><dt>{td.size}</dt><dd>{kb(targetImage.size)} · {td.at0}</dd></div>{/if}
        <div class="wide">
          <dt>esptool</dt>
          {#if firmware.esptool}
            <dd>v{firmware.esptool.version} <span class="muted">{td.esptoolReady}</span></dd>
          {:else}
            <dd class="small">{td.esptoolSetup}</dd>
          {/if}
        </div>
      </dl>
    {/if}
  </Section>
</Advanced>
