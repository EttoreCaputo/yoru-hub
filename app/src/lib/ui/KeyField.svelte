<!--
  One provider's API key: where it comes from, a field to paste a new one
  (checked when saved), a check, and the page that gives keys out.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import type { KeyCheck, KeySource } from "../types";

  let { provider, source, placeholder = "", onchange }: { provider: string; source: KeySource | null; placeholder?: string; onchange?: () => void } = $props();
  const t = $derived(i18n.t.settings);
  let key = $state("");
  let busy = $state(false);
  let check = $state<KeyCheck | null>(null);
  let error = $state("");

  async function save(value: string | null) {
    busy = true;
    error = "";
    check = null;
    try {
      await call("set_api_key", { provider, key: value });
      key = "";
      if (value) check = { state: "valid", error: null };
      onchange?.();
    } catch (e) {
      error = why(e);
    } finally {
      busy = false;
    }
  }

  async function verify() {
    busy = true;
    try {
      check = await call<KeyCheck>("check_api_key", { provider });
    } catch (e) {
      error = why(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="key">
  <div class="state">
    <span class="dot {source ? (check?.state === 'invalid' ? 'wrong' : 'done') : 'wait'}"></span>
    <span>{source === "env" ? t.keyFromEnv(`${provider.toUpperCase()}_API_KEY`) : source === "saved" ? t.keySaved : t.keyMissing}</span>
    {#if busy && !key}<span class="faint">{t.checking}</span>{:else if check}<span class:error-text={check.state !== "valid"} class:dim={check.state === "valid"}>{t[check.state]}</span>{/if}
  </div>
  <form class="row" onsubmit={(e) => (e.preventDefault(), save(key.trim()))}>
    <input type="password" bind:value={key} {placeholder} autocomplete="off" spellcheck="false" aria-label={provider} />
    <button class="btn primary" type="submit" disabled={!key.trim() || busy}>{busy && key ? t.checking : t.saveKey}</button>
    {#if source}<button class="btn quiet" type="button" disabled={busy} onclick={verify}>{t.check}</button>{/if}
    {#if source === "saved"}<button class="btn quiet" type="button" disabled={busy} onclick={() => save(null)}>{t.removeKey}</button>{/if}
    <button class="btn quiet" type="button" onclick={() => call("open_key_page", { provider })}>{t.getKey} ↗</button>
  </form>
  {#if error}<p class="error-text">{error}</p>{/if}
  {#if check?.error && check.state !== "valid"}<p class="error-text">{check.error}</p>{/if}
</div>

<style>
  .key {
    display: grid;
    gap: 10px;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  form {
    flex-wrap: nowrap;
  }
  input {
    flex: 1;
    min-width: 0;
    font-family: var(--mono);
    font-size: 12.5px;
  }
</style>
