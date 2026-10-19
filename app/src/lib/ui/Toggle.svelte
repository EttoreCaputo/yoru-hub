<!-- A switch: on or off, at once. -->
<script lang="ts">
  let { checked, label, disabled = false, onchange }: { checked: boolean; label: string; disabled?: boolean; onchange: (on: boolean) => void } = $props();
</script>

<button class="toggle" class:on={checked} role="switch" aria-checked={checked} aria-label={label} {disabled} onclick={() => onchange(!checked)}>
  <span class="knob"></span>
</button>

<style>
  .toggle {
    position: relative;
    width: 38px;
    height: 22px;
    border-radius: 999px;
    border: 1px solid var(--rim-2);
    background: var(--ink-3);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: background 200ms;
  }
  .toggle:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--dusk);
    transition:
      transform 220ms cubic-bezier(0.3, 1.4, 0.5, 1),
      background 200ms;
  }
  .on {
    border-color: transparent;
    background: var(--pearl-deep);
  }
  .on .knob {
    transform: translateX(16px);
    background: radial-gradient(circle at 35% 30%, #fff, var(--pearl-mid));
  }
</style>
