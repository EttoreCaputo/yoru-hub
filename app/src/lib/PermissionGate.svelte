<!--
  In front of the window while the microphone isn't Yoru's: without it he
  can't hear you from the Mac, so nothing else is offered until it is. Goes
  away by itself once granted (in the prompt or in System Settings).
-->
<script lang="ts">
  import { i18n } from "./i18n/index.svelte";
  import PermissionList from "./ui/PermissionList.svelte";

  const t = $derived(i18n.t.permissions);
</script>

<div class="veil" role="dialog" aria-modal="true" aria-labelledby="gate-title">
  <div class="sheet">
    <h1 id="gate-title">{t.gateTitle}</h1>
    <p class="dim">{t.gateText}</p>
    <PermissionList />
    <p class="faint">{t.later}</p>
  </div>
</div>

<style>
  .veil {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
    backdrop-filter: blur(6px);
  }
  .sheet {
    display: grid;
    gap: 12px;
    width: min(560px, 100%);
    max-height: 100%;
    overflow: auto;
    padding: 28px;
    border-radius: 24px;
    background: var(--ink);
    border: 1px solid var(--rim-2);
    box-shadow: var(--shadow);
  }
  h1 {
    font: 600 var(--fs-title)/1.15 var(--display);
    letter-spacing: -0.01em;
  }
</style>
