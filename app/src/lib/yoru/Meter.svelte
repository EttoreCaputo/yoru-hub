<!-- A thin bar with its name and value: plan usage, the pet's levels. -->
<script lang="ts">
  let { label, value, text, color, note = "" }: { label: string; value: number; text: string; color: string; note?: string } = $props();
  const pct = $derived(Math.max(0, Math.min(1, value)) * 100);
</script>

<div class="meter" style:--c={color}>
  <div class="row">
    <span class="label">{label}</span>
    {#if note}<span class="note">{note}</span>{/if}
    <span class="value">{text}</span>
  </div>
  <div class="track"><div class="fill" style:width="{pct}%"></div></div>
</div>

<style>
  .meter {
    display: grid;
    gap: 6px;
  }
  .row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font: 500 12px/1 var(--sans);
  }
  .label {
    flex: none;
    color: var(--text);
  }
  .note {
    min-width: 0;
    color: var(--faint);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .value {
    flex: none;
    margin-left: auto;
    font: 550 12px/1 var(--mono);
    color: var(--dim);
    font-variant-numeric: tabular-nums;
  }
  .track {
    height: 5px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.06);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: inherit;
    background: var(--c);
    box-shadow: 0 0 10px color-mix(in srgb, var(--c) 60%, transparent);
    transition: width 700ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
</style>
