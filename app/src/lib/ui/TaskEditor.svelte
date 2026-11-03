<!--
  Give Yoru a task, or change one: what to do in words, when (once, every
  day or some weekdays, every month, every few hours) and how far it may go.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { Acting, Task, TaskSchedule } from "../types";
  import Toggle from "./Toggle.svelte";

  let { task, onclose }: { task: Task; onclose: () => void } = $props();
  const t = $derived(i18n.t.tasks);
  // The form edits a copy: the one it was opened with.
  const t0 = untrack(() => task);
  const isNew = !t0.id;

  const tomorrow = () => {
    const d = new Date(Date.now() + 24 * 3600 * 1000);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  };

  let name = $state(t0.name);
  let what = $state(t0.what);
  let repeat = $state(t0.repeat);
  let at = $state("at" in t0 ? t0.at : "09:00");
  let date = $state(t0.repeat === "once" ? t0.date : tomorrow());
  let days = $state<number[]>(t0.repeat === "daily" ? [...t0.days] : []);
  let day = $state(t0.repeat === "monthly" ? t0.day : 1);
  let every = $state(t0.repeat === "hourly" ? t0.every : 3);
  let acting = $state<Acting>(t0.acting);
  let enabled = $state(t0.enabled);
  let error = $state("");
  let saving = $state(false);

  const REPEATS = ["once", "daily", "monthly", "hourly"] as const;
  const ACTINGS: Acting[] = ["never", "ask", "free"];
  const toggleDay = (d: number) => (days = days.includes(d) ? days.filter((x) => x !== d) : [...days, d].sort());

  const schedule = $derived.by((): TaskSchedule => {
    if (repeat === "once") return { repeat, date, at };
    if (repeat === "monthly") return { repeat, day: Math.round(day), at };
    if (repeat === "hourly") return { repeat, every: Math.round(every) };
    return { repeat: "daily", at, days };
  });
  const valid = $derived(what.trim() && (repeat === "hourly" ? every >= 1 && every <= 168 : at && (repeat !== "once" || date) && (repeat !== "monthly" || (day >= 1 && day <= 31))));

  async function save() {
    if (!valid) return;
    saving = true;
    error = "";
    try {
      await store.putTask({ ...schedule, id: t0.id, name: name.trim(), what: what.trim(), enabled, acting });
      onclose();
    } catch (e) {
      error = why(e);
    } finally {
      saving = false;
    }
  }
</script>

<form class="panel editor" onsubmit={(e) => (e.preventDefault(), save())}>
  <h2>{isNew ? t.new : t.editTitle}</h2>

  <label class="stack">{t.what}<textarea rows="3" bind:value={what} placeholder={t.whatPlaceholder}></textarea></label>
  <label class="stack">{t.name}<input type="text" maxlength="60" bind:value={name} placeholder={t.namePlaceholder} /></label>

  <fieldset>
    <legend class="dim">{t.when}</legend>
    <div class="seg" role="group" aria-label={t.when}>
      {#each REPEATS as r (r)}
        <button type="button" class:on={repeat === r} aria-pressed={repeat === r} onclick={() => (repeat = r)}>{t.repeat[r]}</button>
      {/each}
    </div>
    <div class="when">
      {#if repeat === "once"}
        <label class="stack">{t.date}<input type="date" bind:value={date} /></label>
      {:else if repeat === "monthly"}
        <label class="stack">{t.dayOfMonth}<input type="number" min="1" max="31" bind:value={day} /></label>
      {:else if repeat === "hourly"}
        <label class="stack">{t.everyHours}<input type="number" min="1" max="168" bind:value={every} /></label>
      {/if}
      {#if repeat !== "hourly"}
        <label class="stack">{t.at}<input type="time" bind:value={at} /></label>
      {/if}
    </div>
    {#if repeat === "daily"}
      <div class="days" role="group" aria-label={t.days}>
        {#each t.weekdays as label, i (i)}
          <button type="button" class:on={days.includes(i + 1)} aria-pressed={days.includes(i + 1)} onclick={() => toggleDay(i + 1)}>{label}</button>
        {/each}
        <span class="faint">{days.length === 0 || days.length === 7 ? t.everyDay : ""}</span>
      </div>
    {/if}
  </fieldset>

  <fieldset>
    <legend class="dim">{t.acting}</legend>
    <div class="kinds">
      {#each ACTINGS as a (a)}
        <button type="button" class="kind" class:on={acting === a} aria-pressed={acting === a} onclick={() => (acting = a)}>
          <b>{t.actingName[a]}</b><span>{t.actingHint[a]}</span>
        </button>
      {/each}
    </div>
  </fieldset>

  <label class="inline"><span class="dim">{t.enabled}</span><Toggle checked={enabled} label={t.enabled} onchange={(v) => (enabled = v)} /></label>

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
    display: grid;
    gap: 10px;
  }
  legend {
    font-size: 12.5px;
    margin-bottom: 6px;
  }
  .seg,
  .days {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .seg button,
  .days button {
    padding: 6px 12px;
    border-radius: 999px;
    border: 1px solid var(--rim);
    background: var(--ink-2);
    color: var(--haze);
    font: 550 12.5px/1 var(--sans);
    cursor: pointer;
  }
  .days button {
    min-width: 40px;
  }
  .seg button.on,
  .days button.on {
    border-color: var(--pearl-mid);
    background: var(--ink-3);
    color: var(--pearl);
  }
  .days .faint {
    font-size: 12px;
    margin-left: 4px;
  }
  .when {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .when input[type="number"] {
    width: 90px;
  }
  .kinds {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
  }
  .kind {
    display: grid;
    align-content: start;
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
  .inline {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  @media (max-width: 640px) {
    .kinds {
      grid-template-columns: 1fr;
    }
  }
</style>
