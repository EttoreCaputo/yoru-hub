<!--
  A button that asks before it acts, in place: two clicks instead of
  confirm(), which the webview may swallow (it then returns false and the
  button does nothing). With `icon`, the first button is that icon alone,
  its label as the tooltip.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import type { IconName } from "../icons/lucide";
  import Icon from "./Icon.svelte";

  let {
    label,
    question,
    sure,
    icon,
    small = false,
    disabled = false,
    onconfirm,
  }: { label: string; question: string; sure: string; icon?: IconName; small?: boolean; disabled?: boolean; onconfirm: () => unknown } = $props();

  let asking = $state(false);

  async function confirmed() {
    asking = false;
    await onconfirm();
  }
</script>

{#if asking}
  <span class="faint sure">{question}</span>
  <button class="btn quiet" class:small onclick={() => (asking = false)}>{i18n.t.confirm.cancel}</button>
  <button class="btn" class:small onclick={confirmed}>{sure}</button>
{:else if icon}
  <button class="icon-btn" {disabled} onclick={() => (asking = true)} aria-label={label} title={label}><Icon name={icon} size={17} /></button>
{:else}
  <button class="btn quiet" class:small {disabled} onclick={() => (asking = true)}>{label}</button>
{/if}

<style>
  .sure {
    font-size: var(--fs-meta);
    align-self: center;
  }
</style>
