<!--
  The field to talk to Yoru: typed here, answered in the conversation. While
  an answer is on its way the field waits, and says so.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";

  let { autofocus = false, big = false }: { autofocus?: boolean; big?: boolean } = $props();
  const t = $derived(i18n.t.composer);
  const ready = $derived(store.info?.ready ?? false);
  let text = $state("");
  let field: HTMLTextAreaElement | undefined = $state();

  $effect(() => {
    if (autofocus) field?.focus();
  });

  async function send() {
    const said = text.trim();
    if (!said || store.asking || !ready) return;
    text = "";
    await store.ask(said);
    field?.focus();
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }
</script>

<form class="composer" class:big class:busy={!!store.asking} onsubmit={(e) => (e.preventDefault(), send())}>
  <textarea
    bind:this={field}
    bind:value={text}
    rows="1"
    placeholder={store.asking ? t.thinking : ready ? t.placeholder : t.placeholderNoKey}
    disabled={!ready}
    onkeydown={key}
    aria-label={t.placeholder}
  ></textarea>
  <button class="send" type="submit" disabled={!text.trim() || !!store.asking || !ready} aria-label={t.send} title={t.send}>
    {#if store.asking}
      <span class="spin" aria-hidden="true"></span>
    {:else}
      <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 15V5M5.5 9.5 10 5l4.5 4.5" /></svg>
    {/if}
  </button>
</form>
{#if store.askError}<p class="error-text err">{store.askError}</p>{/if}

<style>
  .composer {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 6px 6px 6px 16px;
    border-radius: 22px;
    background: var(--ink-2);
    border: 1px solid var(--rim);
    transition: border-color 200ms;
  }
  .composer:focus-within {
    border-color: var(--pearl-deep);
  }
  .composer.busy {
    border-color: color-mix(in srgb, var(--think) 45%, transparent);
  }
  textarea {
    flex: 1;
    min-height: 32px;
    max-height: 160px;
    padding: 7px 0;
    border: 0;
    background: none;
    resize: none;
    field-sizing: content;
    font-size: 14px;
  }
  .big textarea {
    font-size: 15px;
  }
  textarea:focus {
    border: 0;
  }
  .send {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 0;
    cursor: pointer;
    background: radial-gradient(circle at 32% 28%, #fff, var(--pearl) 45%, var(--pearl-mid));
    flex-shrink: 0;
  }
  .send:disabled {
    background: var(--ink-3);
    cursor: default;
  }
  svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: var(--eye);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .send:disabled svg {
    stroke: var(--dusk);
  }
  .spin {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--think);
    border-right-color: transparent;
    animation: spin 0.9s linear infinite;
  }
  .err {
    margin: 8px 16px 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
