<!--
  memory.md, as Yoru reads it: edited here as Markdown, saved on demand
  (or with Cmd/Ctrl+S). Read again when you come back, since Yoru may have
  added to it meanwhile.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";

  const t = $derived(i18n.t.memory);
  let saved = $state("");
  let text = $state("");
  let error = $state("");
  let justSaved = $state(false);
  const dirty = $derived(text !== saved);

  $effect(() => {
    call<string>("memory").then(
      (m) => (saved = text = m),
      (e) => (error = why(e)),
    );
  });

  async function save() {
    error = "";
    try {
      await call("set_memory", { text });
      saved = text;
      justSaved = true;
      setTimeout(() => (justSaved = false), 1800);
    } catch (e) {
      error = why(e);
    }
  }

  function key(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      void save();
    }
  }

  // The headings and bullets lit, in a layer under the transparent text.
  const lines = $derived(text.split("\n"));
</script>

<div class="page memory">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
    <div class="row">
      {#if dirty}
        <span class="faint small">{t.unsaved}</span>
        <button class="btn quiet" onclick={() => (text = saved)}>{t.revert}</button>
      {:else if justSaved}
        <span class="dim small">{t.saved}</span>
      {/if}
      <button class="btn primary" disabled={!dirty} onclick={save}>{t.save}</button>
    </div>
  </div>

  {#if error}<p class="error-text">{error}</p>{/if}

  <div class="sheet">
    <div class="file mono faint">{t.file}</div>
    <div class="editor">
      <div class="paint mono" aria-hidden="true">
        {#each lines as l, i (i)}<div class:h1={/^# /.test(l)} class:h2={/^## /.test(l)} class:li={/^\s*[-*] /.test(l)}>{l || " "}</div>{/each}
      </div>
      <textarea class="mono" bind:value={text} onkeydown={key} spellcheck="false" aria-label={t.title}></textarea>
    </div>
  </div>
</div>

<style>
  .memory {
    max-width: 820px;
  }
  .small {
    font-size: 12.5px;
  }
  .sheet {
    border-radius: var(--r-lg);
    background: var(--ink);
    border: 1px solid var(--rim);
    overflow: hidden;
  }
  .file {
    padding: 10px 18px;
    border-bottom: 1px solid var(--rim);
    font-size: 11px;
  }
  .editor {
    position: relative;
  }
  .paint,
  textarea {
    margin: 0;
    padding: 18px 20px 40px;
    font-size: 13.5px;
    line-height: 1.7;
    white-space: pre-wrap;
    word-break: break-word;
    tab-size: 2;
  }
  .paint {
    color: var(--haze);
    pointer-events: none;
  }
  .paint .h1 {
    color: var(--pearl);
    font-weight: 700;
  }
  .paint .h2 {
    color: var(--pearl-mid);
    font-weight: 650;
  }
  .paint .li {
    color: var(--pearl);
  }
  /* The real text sits on top, transparent, so the caret and selection are native. */
  textarea {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    min-height: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: transparent;
    caret-color: var(--pearl);
    resize: none;
    overflow: hidden;
  }
  textarea:focus {
    border: 0;
  }
  textarea::selection {
    color: transparent;
    background: color-mix(in srgb, var(--pearl-mid) 30%, transparent);
  }
</style>
