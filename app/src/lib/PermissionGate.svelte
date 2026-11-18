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
  .sheet {
    display: grid;
    gap: 12px;
    padding: 28px;
  }
  h1 {
    font: 600 var(--fs-title)/1.15 var(--display);
    letter-spacing: -0.01em;
  }
</style>
