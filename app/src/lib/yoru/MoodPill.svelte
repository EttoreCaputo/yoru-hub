<!-- How Yoru feels, from its `get_pet`: the mood, its three levels and what moved them lately. -->
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
</script>

<Pill title={t.title} accent={mood?.color ?? "#8a8fa0"} chip={words?.label ?? null} action={{ label: t.more, onclick: () => settingsNav.show("pet") }}>
  {#if pet && words && mood}
    <p class="says">{pet.away ? tp.away : words.says}</p>
    <div class="levels">
      <Meter label={tp.levels.energy.name} value={pet.energy} text={String(Math.round(pet.energy * 100))} color="#ffd040" />
      <Meter label={tp.levels.happiness.name} value={pet.happiness} text={String(Math.round(pet.happiness * 100))} color="#40e080" />
      <Meter label={tp.levels.affection.name} value={pet.affection} text={String(Math.round(pet.affection * 100))} color="#ff5aa8" />
    </div>
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
  .levels {
    display: grid;
    gap: 10px;
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
