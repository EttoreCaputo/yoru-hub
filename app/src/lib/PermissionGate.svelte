<!--
  In front of the window while the microphone isn't Yoru's: without it he
  can't hear you from the Mac, so nothing else is offered until it is.
  Once it's granted the sheet stays, so the other permissions can be given
  too, until Continue.
-->
<script lang="ts">
  import { i18n } from "./i18n/index.svelte";
  import { store } from "./store.svelte";
  import PermissionList from "./ui/PermissionList.svelte";

  let { onclose }: { onclose: () => void } = $props();
  const t = $derived(i18n.t.permissions);
</script>

<div class="veil" role="dialog" aria-modal="true" aria-labelledby="gate-title">
  <div class="sheet">
    <h1 id="gate-title">{t.gateTitle}</h1>
    <p class="dim">{t.gateText}</p>
    <PermissionList />
    <footer>
      <p class="faint">{store.micMissing ? t.later : t.gateDone}</p>
      <button class="btn primary" disabled={store.micMissing} onclick={onclose}>{t.continue}</button>
    </footer>
  </div>
</div>

<style>
  .sheet {
    display: grid;
    gap: 12px;
    width: min(620px, 100%);
    padding: 28px;
  }
  h1 {
    font: 600 var(--fs-title)/1.15 var(--display);
    letter-spacing: -0.01em;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-top: 6px;
  }
  footer p {
    flex: 1;
    margin: 0;
  }
</style>
