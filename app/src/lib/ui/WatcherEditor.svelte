<!--
  Add or change a watcher: a connector's read tool with fixed arguments, or a
  task in words; how often; what matters in what it finds.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Urgency, Watcher } from "../types";
  import Toggle from "./Toggle.svelte";

  let { watcher, onclose }: { watcher: Watcher; onclose: () => void } = $props();
  const t = $derived(i18n.t.watcher);
  // The form edits a copy: the one it was opened with.
  const w0 = untrack(() => watcher);
  const isNew = !w0.id;

  let name = $state(w0.name);
  let kind = $state(w0.kind);
  let tool = $state(w0.kind === "tool" ? w0.tool : "");
  let args = $state(w0.kind === "tool" ? JSON.stringify(w0.arguments ?? {}, null, 2) : "{}");
  let task = $state(w0.kind === "task" ? w0.task : "");
  let every = $state(w0.every_min);
  let instructions = $state(w0.instructions);
  let maxUrgency = $state<Urgency>(w0.max_urgency);
  let enabled = $state(w0.enabled);
  let error = $state("");
  let saving = $state(false);

  // Only tools that read: a watcher runs with nobody there to say yes.
  const readTools = $derived(
    store.connectors.flatMap((c) => c.tools.filter((x) => x.enabled && !x.acts).map((x) => ({ value: x.qualified_name, label: `${c.name} · ${x.title ?? x.name}`, description: x.description }))),
  );
  const picked = $derived(readTools.find((x) => x.value === tool));
  const parsed = $derived.by(() => {
    try {
      return { ok: true as const, value: JSON.parse(args || "{}") };
    } catch {
      return { ok: false as const };
    }
  });
  const valid = $derived(name.trim() && every >= 5 && (kind === "task" ? task.trim() : tool && parsed.ok));

  async function save() {
    if (!valid) return;
    saving = true;
    error = "";
    const base = { id: w0.id, name: name.trim(), every_min: Math.round(every), enabled, instructions: instructions.trim(), max_urgency: maxUrgency };
    const w: Watcher = kind === "tool" ? { ...base, kind: "tool", tool, arguments: parsed.ok ? parsed.value : {} } : { ...base, kind: "task", task: task.trim() };
    try {
      await store.putWatcher(w);
      onclose();
    } catch (e) {
      error = why(e);
    } finally {
      saving = false;
    }
  }
</script>

<form class="panel editor" onsubmit={(e) => (e.preventDefault(), save())}>
  <h2>{isNew ? t.new : t.edit}</h2>

  <label class="stack">{t.name}<input type="text" bind:value={name} placeholder={t.namePlaceholder} /></label>

  <fieldset>
    <legend class="dim">{t.kind}</legend>
    <div class="kinds">
      <button type="button" class="kind" class:on={kind === "tool"} aria-pressed={kind === "tool"} onclick={() => (kind = "tool")}>
        <b>{t.tool}</b><span>{t.toolHint}</span>
      </button>
      <button type="button" class="kind" class:on={kind === "task"} aria-pressed={kind === "task"} onclick={() => (kind = "task")}>
        <b>{t.task}</b><span>{t.taskHint}</span>
      </button>
    </div>
  </fieldset>

  {#if kind === "tool"}
    {#if readTools.length}
      <label class="stack">
        {t.pickTool}
        <select bind:value={tool}>
          <option value="" disabled>—</option>
          {#each readTools as x (x.value)}<option value={x.value}>{x.label}</option>{/each}
          {#if tool && !picked}<option value={tool}>{tool}</option>{/if}
        </select>
      </label>
      {#if picked?.description}<p class="faint desc">{picked.description}</p>{/if}
      <label class="stack">
        {t.arguments}
        <textarea class="mono" rows="4" bind:value={args} spellcheck="false"></textarea>
      </label>
      {#if !parsed.ok}<p class="error-text">{t.badJson}</p>{/if}
    {:else}
      <p class="dim">{t.noReadTools}</p>
    {/if}
  {:else}
    <label class="stack">{t.taskText}<textarea rows="3" bind:value={task} placeholder={t.taskPlaceholder}></textarea></label>
  {/if}

  <div class="three">
    <label class="stack">{t.every}<input type="number" min="5" step="5" bind:value={every} /></label>
    <label class="stack">
      {t.maxUrgency}
      <select bind:value={maxUrgency}>
        {#each ["low", "normal", "high", "urgent"] as const as u (u)}<option value={u}>{i18n.t.inbox.urgency[u]}</option>{/each}
      </select>
    </label>
    <label class="stack on">{t.enabled}<Toggle checked={enabled} label={t.enabled} onchange={(v) => (enabled = v)} /></label>
  </div>

  <label class="stack">{t.instructions}<input type="text" bind:value={instructions} placeholder={t.instructionsPlaceholder} /></label>

  {#if error}<p class="error-text">{error}</p>{/if}
  <div class="row">
    <button class="btn primary" type="submit" disabled={!valid || saving}>{t.save}</button>
    <button class="btn quiet" type="button" onclick={onclose}>{t.cancel}</button>
  </div>
</form>

<style>
  .editor {
    display: grid;
    gap: 14px;
    margin-bottom: 14px;
    border-color: var(--rim-2);
  }
  h2 {
    font: 600 17px/1.2 var(--display) !important;
  }
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
  }
  legend {
    font-size: 12.5px;
    margin-bottom: 6px;
  }
  .kinds {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .kind {
    display: grid;
    gap: 4px;
    padding: 12px 14px;
    border-radius: var(--r);
    border: 1px solid var(--rim);
    background: var(--ink-2);
    color: var(--pearl);
    text-align: left;
    cursor: pointer;
    font: inherit;
  }
  .kind span {
    color: var(--haze);
    font-size: 12.5px;
    line-height: 1.4;
  }
  .kind.on {
    border-color: var(--pearl-mid);
    background: var(--ink-3);
  }
  .desc {
    margin-top: -6px;
    font-size: 12.5px;
  }
  .three {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: 12px;
    align-items: end;
  }
  .three input[type="number"] {
    width: 100%;
  }
  .stack.on {
    justify-items: start;
    padding-bottom: 7px;
  }
</style>
