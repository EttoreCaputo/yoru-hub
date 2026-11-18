<!--
  The field to talk to Yoru: typed here, answered in the conversation. While
  an answer is on its way the field waits, and says so. With `mic`, a button
  talks to him through the Mac's microphone instead: click to start, click
  again to send what you said; while he answers aloud, it stops him.
  `conversation` is the one it goes on: an earlier one is opened again
  first, and `new` starts one (in a tab of its own).
-->
<script lang="ts">
  import { call } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import Icon from "./Icon.svelte";

  let {
    autofocus = false,
    big = false,
    mic = false,
    conversation,
    placeholder,
  }: { autofocus?: boolean; big?: boolean; mic?: boolean; conversation?: string; placeholder?: string } = $props();
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
    await store.ask(said, false, await target());
    field?.focus();
  }

  /** The conversation to go on: a new one made, an earlier one opened again. */
  async function target(): Promise<string | undefined> {
    if (conversation === "new") return await store.newChat();
    if (conversation && conversation !== store.currentChat) {
      await call("open_conversation", { id: conversation });
      store.currentChat = conversation;
    }
    return conversation;
  }

  const listening = $derived(store.talking || store.macVoice === "listening" || store.macVoice === "starting");
  const speaking = $derived(!store.talking && store.macVoice === "speaking");
  async function micClick() {
    if (speaking) return store.hush();
    // What's said goes to the conversation Yoru is in: this one, first.
    if (!store.talking) await target();
    store.talk(!store.talking);
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
      class="icon-btn round mic"
      class:on={listening}
      type="button"
      style:--level={store.macLevel}
      disabled={!ready}
      onclick={micClick}
      aria-label={speaking ? t.hush : listening ? t.micStop : t.micStart}
      title={speaking ? t.hush : listening ? t.micStop : t.micStart}
    >
      {#if speaking}
        <Icon name="square" size={16} stroke={2} />
      {:else}
        <Icon name="mic" size={17} />
      {/if}
    </button>
  {/if}
  <textarea
    bind:this={field}
    bind:value={text}
    rows="1"
    placeholder={listening ? t.listening : store.asking ? t.thinking : ready ? (placeholder ?? t.placeholder) : t.placeholderNoKey}
    disabled={!ready}
    onkeydown={key}
    aria-label={t.placeholder}
  ></textarea>
  <button class="icon-btn round primary send" type="submit" disabled={!text.trim() || !!store.asking || !ready} aria-label={t.send} title={t.send}>
    {#if store.asking}
      <span class="spin" aria-hidden="true"></span>
    {:else}
      <Icon name="arrow-up" size={16} stroke={2.25} />
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
  .mic {
    margin-left: -10px;
    background: var(--ink-3);
    transition:
      background 160ms,
      box-shadow 80ms;
  }
  /* On: green, with a ring as loud as the voice it hears. */
  .mic.on {
    background: var(--done);
    box-shadow: 0 0 0 calc(2px + var(--level, 0) * 8px) color-mix(in srgb, var(--done) 30%, transparent);
  }
  .mic.on {
    color: var(--eye);
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
