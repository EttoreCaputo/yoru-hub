<!--
  An answer of Yoru's written as Markdown (lib/markdown.ts): headings, lists,
  tables, code, and ```mermaid diagrams drawn once Mermaid has loaded (the
  code stays if it can't be read). Links open in the browser, never here.
  `dark` is for the notch, which is always dark; elsewhere the theme decides.
-->
<script lang="ts">
  import { call } from "../api";
  import { look } from "../look.svelte";
  import { diagram, markdown } from "../markdown";

  let { text, dark }: { text: string; dark?: boolean } = $props();
  const html = $derived(markdown(text));
  const isDark = $derived(dark ?? look.dark);
  let box = $state<HTMLElement>();

  // Each diagram in the answer, drawn for the theme on screen.
  $effect(() => {
    void html;
    const night = isDark;
    for (const el of box?.querySelectorAll<HTMLElement>(".mermaid[data-src]") ?? []) {
      const raw = el.dataset.src ?? "";
      diagram(decodeURIComponent(raw), night).then((svg) => {
        if (!svg || !el.isConnected || el.dataset.src !== raw) return;
        el.innerHTML = svg;
        el.classList.add("drawn");
      });
    }
  });

  function click(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
    if (!a) return;
    e.preventDefault();
    call("open_web", { url: a.getAttribute("href") ?? "" }).catch(() => {});
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="md" class:notch={dark} bind:this={box} onclick={click}>{@html html}</div>

<style>
  .md {
    font-size: inherit;
    line-height: 1.55;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .md :global(> :first-child) {
    margin-top: 0;
  }
  .md :global(> :last-child) {
    margin-bottom: 0;
  }
  .md :global(p),
  .md :global(ul),
  .md :global(ol),
  .md :global(pre),
  .md :global(table),
  .md :global(blockquote),
  .md :global(.mermaid) {
    margin: 0 0 0.7em;
  }
  .md :global(h1),
  .md :global(h2),
  .md :global(h3),
  .md :global(h4),
  .md :global(h5),
  .md :global(h6) {
    margin: 1.1em 0 0.45em;
    font-family: var(--display);
    font-weight: 650;
    line-height: 1.3;
  }
  .md :global(h1) {
    font-size: 1.35em;
  }
  .md :global(h2) {
    font-size: 1.2em;
  }
  .md :global(h3) {
    font-size: 1.08em;
  }
  .md :global(h4),
  .md :global(h5),
  .md :global(h6) {
    font-size: 1em;
  }
  .md :global(ul),
  .md :global(ol) {
    padding-left: 1.5em;
  }
  .md :global(li) {
    margin: 0.2em 0;
  }
  .md :global(li > ul),
  .md :global(li > ol),
  .md :global(li > p) {
    margin-bottom: 0.2em;
  }
  .md :global(li::marker) {
    color: var(--haze);
  }
  .md :global(strong) {
    font-weight: 650;
  }
  .md :global(a) {
    color: var(--think);
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--think) 40%, transparent);
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .md :global(code) {
    font-family: var(--mono);
    font-size: 0.86em;
    padding: 0.1em 0.35em;
    border-radius: 5px;
    background: var(--ink-3);
  }
  .md :global(pre) {
    padding: 10px 12px;
    border-radius: var(--r-md);
    background: var(--ink-2);
    box-shadow: 0 0 0 1px var(--rim);
    overflow-x: auto;
  }
  .md :global(pre code) {
    padding: 0;
    background: none;
    font-size: 0.84em;
    line-height: 1.5;
    white-space: pre;
  }
  .md :global(blockquote) {
    padding: 0 0 0 12px;
    border-left: 3px solid var(--rim-2);
    color: var(--haze);
  }
  .md :global(hr) {
    border: none;
    border-top: 1px solid var(--rim-2);
    margin: 1em 0;
  }
  .md :global(table) {
    display: block;
    max-width: 100%;
    overflow-x: auto;
    border-collapse: collapse;
    font-size: 0.93em;
  }
  .md :global(th),
  .md :global(td) {
    padding: 5px 10px;
    border: 1px solid var(--rim-2);
    text-align: left;
    vertical-align: top;
  }
  .md :global(th) {
    background: var(--ink-2);
    font-weight: 600;
  }
  .md :global(.mermaid.drawn) {
    padding: 12px;
    border-radius: var(--r-md);
    background: var(--ink-2);
    box-shadow: 0 0 0 1px var(--rim);
    overflow-x: auto;
    text-align: center;
  }
  .md :global(.mermaid svg) {
    max-width: 100%;
    height: auto;
  }
  /* The notch: white on black whatever the window's theme. */
  .notch.md :global(code),
  .notch.md :global(pre),
  .notch.md :global(th),
  .notch.md :global(.mermaid.drawn) {
    background: rgba(255, 255, 255, 0.08);
    box-shadow: none;
  }
  .notch.md :global(th),
  .notch.md :global(td),
  .notch.md :global(blockquote) {
    border-color: rgba(255, 255, 255, 0.18);
  }
</style>
