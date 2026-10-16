<!--
  How Yoru feels, from its `get_pet`: the mood, what it is doing, its needs
  (firmware 0.6.0) or its three levels before that, a firefly to feed it,
  and what moved it lately.
-->
<script lang="ts">
  import Pill from "./Pill.svelte";
  import Meter from "./Meter.svelte";
  import { monitor } from "../monitor.svelte";
  import { MOODS } from "../petMood";
  import { i18n } from "../i18n/index.svelte";
  import { settingsNav } from "../settings/nav.svelte";

  const t = $derived(i18n.t.yoru.mood);
  const tp = $derived(i18n.t.pet);
  const pet = $derived(monitor.pet);
  const mood = $derived(pet ? (MOODS[pet.mood] ?? MOODS.content) : null);
  const words = $derived(pet ? (tp.moods[pet.mood] ?? tp.moods.content) : null);
  const reactions = $derived(tp.reactions as Record<string, string>);
  const NEEDS = [
    ["rest", "#7a9cff"],
    ["food", "#ffa030"],
    ["fun", "#40e080"],
    ["company", "#ff5aa8"],
    ["curiosity", "#30d5f0"],
  ] as const;

  let feeding = $state(false);
  let fed = $state(false);
  let feedError = $state<string | null>(null);
  async function feed() {
    feeding = true;
    feedError = null;
    try {
      await monitor.yoruFeed();
      fed = true;
      setTimeout(() => (fed = false), 4000);
    } catch (e) {
      // The board says why: asleep, the menu is up, a conversation is on.
      feedError = String(e);
    } finally {
      feeding = false;
    }
  }
</script>

<Pill title={t.title} accent={mood?.color ?? "#8a8fa0"} chip={words?.label ?? null} action={{ label: t.more, onclick: () => settingsNav.show("pet") }}>
  {#if pet && words && mood}
    <p class="says">{pet.away ? tp.away : words.says}</p>
    {#if pet.activity}<p class="doing"><span class="dot" style:background={mood.color}></span>{tp.activities[pet.activity] ?? pet.activity}</p>{/if}
    {#if pet.needs}
      <div class="levels" aria-label={t.needs}>
        {#each NEEDS as [key, color] (key)}
          <Meter label={tp.needs[key]} value={pet.needs[key]} text={String(Math.round(pet.needs[key] * 100))} {color} />
        {/each}
      </div>
      <div class="feed">
        <button onclick={feed} disabled={feeding}>{t.feed}</button>
        {#if feedError}<span class="error">{feedError}</span>{:else if fed}<span class="ok">{t.feeding}</span>{/if}
      </div>
    {:else}
      <div class="levels">
        <Meter label={tp.levels.energy.name} value={pet.energy} text={String(Math.round(pet.energy * 100))} color="#ffd040" />
        <Meter label={tp.levels.happiness.name} value={pet.happiness} text={String(Math.round(pet.happiness * 100))} color="#40e080" />
        <Meter label={tp.levels.affection.name} value={pet.affection} text={String(Math.round(pet.affection * 100))} color="#ff5aa8" />
      </div>
    {/if}
    {#if pet.recent.length}
      <ul>
        {#each pet.recent.slice(0, 3) as r, i (i)}
          <li><span>{reactions[r.what] ?? r.what}</span><small>{tp.ago(r.ago_s)}</small></li>
        {/each}
      </ul>
    {/if}
  {:else}
    <p class="says">{t.none}</p>
  {/if}
</Pill>

<style>
  .says {
    margin: 0;
    font: 450 12.5px/1.45 var(--sans);
    color: var(--dim);
  }
  .doing {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 0;
    font: 550 12px/1.2 var(--sans);
    color: var(--text);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
  .levels {
    display: grid;
    gap: 10px;
  }
  .feed {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 26px;
  }
  .feed button {
    flex: none;
    padding: 7px 12px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, #ffa030 40%, transparent);
    background: color-mix(in srgb, #ffa030 12%, transparent);
    color: var(--text);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
    transition: background 160ms;
  }
  .feed button:hover {
    background: color-mix(in srgb, #ffa030 22%, transparent);
  }
  .feed button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .feed span {
    font: 450 11.5px/1.3 var(--sans);
  }
  .error {
    color: var(--hot);
  }
  .ok {
    color: var(--dim);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 10px 0 0;
    border-top: 1px solid var(--line);
    display: grid;
    gap: 7px;
  }
  li {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    font: 450 12px/1.2 var(--sans);
  }
  small {
    color: var(--faint);
    font-size: 11px;
    white-space: nowrap;
  }
</style>
