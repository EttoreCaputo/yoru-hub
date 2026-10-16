<!--
  A Yoru's own settings, built from its `get_settings` table: a 0/1 setting
  is a switch, any other range a slider. Each change goes to the board at
  once (`set_setting`); the table is read again when the board shown changes.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { monitor, type YoruSetting, type YoruSettings } from "../monitor.svelte";
  import Toggle from "./Toggle.svelte";

  const td = $derived(i18n.t.settings.device);

  let table = $state<YoruSettings | null>(null);
  let error = $state("");
  let busy = $state<string | null>(null);

  $effect(() => {
    // Again for each board shown, and once it's back.
    const board = monitor.boardId;
    if (monitor.link !== "connected") return;
    let alive = true;
    table = null;
    error = "";
    monitor
      .yoruSettings()
      .then((t) => alive && board === monitor.boardId && (table = t))
      .catch((e) => alive && (error = String(e)));
    return () => {
      alive = false;
    };
  });

  const rows = $derived(
    table ? Object.entries(table).filter((e): e is [string, YoruSetting] => typeof e[1] === "object" && e[1] !== null && "value" in e[1]) : [],
  );
  const wakeWord = $derived(typeof table?.wake_word === "string" ? table.wake_word : null);
  const label = (name: string) => td.yoruNames[name] ?? name.replaceAll("_", " ");

  async function set(name: string, value: number) {
    const s = table?.[name];
    if (typeof s !== "object") return;
    const before = s.value;
    s.value = value;
    busy = name;
    error = "";
    try {
      await monitor.yoruSetSetting(name, value);
    } catch (e) {
      s.value = before;
      error = String(e);
    } finally {
      busy = null;
    }
  }
</script>

{#if !table && !error}
  <p class="empty">{td.yoruReading}</p>
{:else if table}
  {#if wakeWord}
    <dl class="facts"><div><dt>{td.wakeWord}</dt><dd>{wakeWord}</dd></div></dl>
  {/if}
  {#each rows as [name, s] (name)}
    {#if s.min === 0 && s.max === 1}
      <Toggle checked={s.value === 1} label={label(name)} disabled={busy === name} onchange={(on) => set(name, on ? 1 : 0)} />
    {:else}
      <label class="range">
        <span class="rname">{label(name)}</span>
        <input type="range" min={s.min} max={s.max} step="1" value={s.value} disabled={busy === name} onchange={(e) => set(name, Number(e.currentTarget.value))} />
        <span class="rval">{s.value}</span>
      </label>
    {/if}
  {/each}
  <div class="actions">
    <button class="btn" onclick={() => monitor.yoruShowMenu(true).catch((e) => (error = String(e)))}>{td.yoruOpenMenu}</button>
  </div>
{/if}
{#if error}<p class="hint error">{error}</p>{/if}

<style>
  .range {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 160px 36px;
    align-items: center;
    gap: 12px;
    padding: 6px 0;
  }
  .rname {
    color: var(--text);
  }
  .rval {
    font: 500 11.5px/1 var(--mono);
    color: var(--dim);
    text-align: right;
  }
</style>
