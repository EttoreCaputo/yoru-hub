<!--
  The field to talk to Yoru: typed here, answered in the conversation. While
  an answer is on its way the field waits, and says so. With `mic`, a button
  talks to him through the Mac's microphone instead: click to start, click
  again to send what you said; while he answers aloud, it stops him.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";

  let { autofocus = false, big = false, mic = false }: { autofocus?: boolean; big?: boolean; mic?: boolean } = $props();
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

  const listening = $derived(store.talking || store.macVoice === "listening" || store.macVoice === "starting");
  const speaking = $derived(!store.talking && store.macVoice === "speaking");
  function micClick() {
    if (speaking) store.hush();
    else store.talk(!store.talking);
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }
</script>

<form class="composer" class:big class:busy={!!store.asking} class:hearing={listening} onsubmit={(e) => (e.preventDefault(), send())}>
  {#if mic}
    <button
      class="mic"
      class:on={listening}
      type="button"
      style:--level={store.macLevel}
      disabled={!ready}
      onclick={micClick}
      aria-label={speaking ? t.hush : listening ? t.micStop : t.micStart}
      title={speaking ? t.hush : listening ? t.micStop : t.micStart}
    >
      {#if speaking}
        <svg viewBox="0 0 20 20" aria-hidden="true"><rect x="6" y="6" width="8" height="8" rx="1.5" /></svg>
      {:else}
        <svg viewBox="0 0 20 20" aria-hidden="true"><rect x="7.5" y="2.5" width="5" height="9" rx="2.5" /><path d="M4.5 9.5a5.5 5.5 0 0 0 11 0M10 15v2.5" /></svg>
      {/if}
    </button>
  {/if}
  <textarea
    bind:this={field}
    bind:value={text}
    rows="1"
    placeholder={listening ? t.listening : store.asking ? t.thinking : ready ? t.placeholder : t.placeholderNoKey}
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
{#if mic && store.macError}<p class="error-text err">{store.macError}</p>{/if}

<style>
  .composer {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 6px 6px 6px 16px;
    border-radius: 22px;
    background: var(--field);
    border: 1px solid var(--rim);
    box-shadow: var(--lift);
    transition: border-color 200ms;
  }
  .composer:focus-within {
    border-color: var(--pearl-deep);
  }
  .composer.hearing {
    border-color: color-mix(in srgb, var(--done) 55%, transparent);
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
    /* It wraps: a horizontal scrollbar would only draw a bar under the text. */
    overflow-x: hidden;
    font-size: var(--fs-body);
  }
  .big textarea {
    font-size: var(--fs-head);
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
  :global(:root[data-theme="light"]) .send:not(:disabled) {
    background: var(--pearl);
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
  .mic {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    margin-left: -10px;
    border-radius: 50%;
    border: 0;
    background: var(--ink-3);
    cursor: pointer;
    flex-shrink: 0;
    transition:
      background 160ms,
      box-shadow 80ms;
  }
  .mic svg {
    stroke: var(--haze);
  }
  .mic:hover svg {
    stroke: var(--pearl);
  }
  /* On: green, with a ring as loud as the voice it hears. */
  .mic.on {
    background: var(--done);
    box-shadow: 0 0 0 calc(2px + var(--level, 0) * 8px) color-mix(in srgb, var(--done) 30%, transparent);
  }
  .mic.on svg {
    stroke: var(--eye);
  }
  .mic:disabled {
    cursor: default;
    opacity: 0.5;
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
