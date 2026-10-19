<!--
  The services Yoru reaches: the connected ones with their state, sign-in and
  tools; the catalog to add more, where Connect opens the service's sign-in
  page and nothing else is asked (the user's own OAuth client is folded
  away under Advanced, for services this build has no client for); and any
  other remote MCP server by its URL.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ConnectorStatus, Preset } from "../types";
  import ConnectorCard from "../ui/ConnectorCard.svelte";
  import ServiceIcon from "../ui/ServiceIcon.svelte";

  const t = $derived(i18n.t.connectors);
  const added = $derived(new Set(store.connectors.map((c) => c.id)));
  const oneClick = (p: Preset) => p.registration !== "own_client" || p.app_client;
  const catalog = $derived(store.presets.filter((p) => !added.has(p.id) && oneClick(p)));
  /** Services only the user's own client can sign in to, in this build. */
  const needsClient = $derived(store.presets.filter((p) => !added.has(p.id) && !oneClick(p)));

  /** The preset whose Advanced form is open. */
  let opening = $state<string | null>(null);
  let clientId = $state("");
  let clientSecret = $state("");
  let busy = $state<string | null>(null);
  let error = $state<{ id: string; text: string } | null>(null);

  let custom = $state({ id: "", name: "", url: "" });
  let customOpen = $state(false);

  function advanced(p: Preset) {
    opening = opening === p.id ? null : p.id;
    clientId = "";
    clientSecret = "";
  }

  /** Add the service and go straight to its sign-in page; the user's own
   *  client only when the Advanced form is open. */
  async function connect(p: Preset) {
    const own = opening === p.id && clientId.trim() !== "";
    busy = p.id;
    error = null;
    try {
      await call<ConnectorStatus>("add_connector", { config: null, preset: p.id, clientId: own ? clientId : null, clientSecret: own ? clientSecret || null : null });
      opening = null;
      await store.refreshConnectors();
      // The card shows "waiting for the browser" while the user signs in there.
      if (p.registration !== "none") void signIn(p.id);
    } catch (e) {
      error = { id: p.id, text: why(e) };
    } finally {
      busy = null;
    }
  }

  async function addCustom() {
    busy = "custom";
    error = null;
    try {
      const id = custom.id.trim().toLowerCase();
      await call("add_connector", { config: { id, name: custom.name.trim() || id, transport: { type: "http", url: custom.url.trim() }, enabled: true }, preset: null, clientId: null, clientSecret: null });
      custom = { id: "", name: "", url: "" };
      customOpen = false;
      await store.refreshConnectors();
      void signIn(id);
    } catch (e) {
      error = { id: "custom", text: why(e) };
    } finally {
      busy = null;
    }
  }

  let signingIn = $state(new Set<string>());
  async function signIn(id: string) {
    signingIn = new Set([...signingIn, id]);
    try {
      await call("login_connector", { id });
    } catch {
      // The card shows the connector's own state and error.
    } finally {
      signingIn = new Set([...signingIn].filter((x) => x !== id));
      await store.refreshConnectors();
    }
  }

  const redirect = (p: Preset) => `http://127.0.0.1${p.redirect_port ? `:${p.redirect_port}` : ""}/callback`;
  const openDocs = (url: string) => call("open_web", { url }).catch(() => {});
  const how = (p: Preset) => (p.registration === "none" ? t.noSignIn : oneClick(p) ? t.dynamic : t.ownClient);
</script>

<div class="page">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  {#if store.connectorsError}
    <p class="panel error-text">{t.notRunning} {store.connectorsError}</p>
  {/if}

  <h2 class="section-title">{t.yours}</h2>
  {#if store.connectors.length}
    <div class="mine">
      {#each store.connectors as c (c.id)}
        <ConnectorCard {c} signingIn={signingIn.has(c.id)} onsignin={() => signIn(c.id)} />
      {/each}
    </div>
  {:else}
    <div class="panel empty"><b>{t.none}</b><p>{t.noneHint}</p></div>
  {/if}

  <h2 class="section-title">{t.catalog}</h2>
  {#snippet card(p: Preset)}
    <article class="preset" class:open={opening === p.id}>
      <div class="top">
        <ServiceIcon id={p.id} name={p.name} url={p.url} size={34} />
        <div class="name">
          <h3>{p.name}</h3>
          <span class="faint">{how(p)}</span>
        </div>
        {#if oneClick(p) && opening !== p.id}
          <button class="btn small" disabled={busy === p.id} onclick={() => connect(p)}>{busy === p.id ? t.opening : t.connect}</button>
        {/if}
      </div>
      <p class="notes dim">{p.notes}</p>
      {#if p.registration === "own_client"}
        <button class="btn quiet small advanced" aria-expanded={opening === p.id} onclick={() => advanced(p)}>
          <span class="chev" class:down={opening === p.id} aria-hidden="true">›</span>{t.advanced}
        </button>
      {/if}
      {#if opening === p.id}
        <form class="client" onsubmit={(e) => (e.preventDefault(), connect(p))}>
          <p class="dim">{p.app_client ? t.advancedHint : t.advancedNeeded}</p>
          <p class="dim">{p.setup}</p>
          <button type="button" class="btn quiet small docs" onclick={() => openDocs(p.docs)}>{t.ownClientHow} ↗</button>
          <p class="mono faint">{t.redirect(redirect(p))}</p>
          <label class="stack">{t.clientId}<input type="text" bind:value={clientId} autocomplete="off" spellcheck="false" /></label>
          {#if p.needs_client_secret}
            <label class="stack">{t.clientSecret}<input type="password" bind:value={clientSecret} autocomplete="off" /></label>
          {/if}
          <div class="row">
            <button class="btn primary small" type="submit" disabled={!clientId.trim() || (p.needs_client_secret && !clientSecret.trim()) || busy === p.id}>{t.connectOwn}</button>
            <button class="btn quiet small" type="button" onclick={() => (opening = null)}>{t.cancel}</button>
          </div>
        </form>
      {/if}
      {#if error?.id === p.id}<p class="error-text">{error.text}</p>{/if}
    </article>
  {/snippet}

  <div class="catalog">
    {#each catalog as p (p.id)}
      {@render card(p)}
    {/each}

    <article class="preset custom" class:open={customOpen}>
      <div class="top">
        <span class="plus" aria-hidden="true">+</span>
        <div class="name">
          <h3>{t.custom}</h3>
          <span class="faint">MCP</span>
        </div>
        {#if !customOpen}<button class="btn small" onclick={() => (customOpen = true)}>{t.add}</button>{/if}
      </div>
      <p class="notes dim">{t.customHint}</p>
      {#if customOpen}
        <form class="client" onsubmit={(e) => (e.preventDefault(), addCustom())}>
          <label class="stack">{t.customUrl}<input type="url" bind:value={custom.url} placeholder="https://…/mcp" spellcheck="false" /></label>
          <div class="two">
            <label class="stack">{t.customName}<input type="text" bind:value={custom.name} /></label>
            <label class="stack">{t.customId}<input type="text" bind:value={custom.id} placeholder="my-server" spellcheck="false" /></label>
          </div>
          <div class="row">
            <button class="btn primary small" type="submit" disabled={!custom.url.trim() || !custom.id.trim() || busy === "custom"}>{t.add}</button>
            <button class="btn quiet small" type="button" onclick={() => (customOpen = false)}>{t.cancel}</button>
          </div>
        </form>
      {/if}
      {#if error?.id === "custom"}<p class="error-text">{error.text}</p>{/if}
    </article>
  </div>

  {#if needsClient.length}
    <h2 class="section-title">{t.needsClient}</h2>
    <p class="faint section-hint">{t.needsClientHint}</p>
    <div class="catalog">
      {#each needsClient as p (p.id)}
        {@render card(p)}
      {/each}
    </div>
  {/if}
</div>

<style>
  .mine {
    display: grid;
    gap: 12px;
  }
  .mine :global(.panel + .panel) {
    margin-top: 0;
  }
  .catalog {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 12px;
    align-items: start;
  }
  .preset {
    padding: 16px;
    border-radius: var(--r-lg);
    background: var(--ink);
    border: 1px solid var(--rim);
    transition: border-color 160ms;
  }
  .preset:hover,
  .preset.open {
    border-color: var(--rim-2);
  }
  .preset.open {
    grid-column: 1 / -1;
  }
  .preset.custom {
    border-style: dashed;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .name {
    flex: 1;
    min-width: 0;
  }
  .name h3 {
    font: 600 14.5px/1.2 var(--sans);
  }
  .name span {
    font-size: 11.5px;
  }
  /* The "another server" tile: a dashed moon. */
  .plus {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    border: 1px dashed var(--rim-2);
    color: var(--pearl-mid);
    font: 650 17px/1 var(--display);
    flex-shrink: 0;
  }
  .section-hint {
    margin: -4px 0 12px;
    font-size: 12.5px;
  }
  .advanced {
    margin-top: 6px;
    padding: 0;
    height: auto;
    gap: 4px;
    color: var(--dusk);
  }
  .chev {
    display: inline-block;
    transition: transform 160ms;
  }
  .chev.down {
    transform: rotate(90deg);
  }
  .notes {
    margin-top: 10px;
    font-size: 12.5px;
    line-height: 1.45;
  }
  /* The whole story once it's open; three lines while browsing. */
  .preset:not(.open) .notes {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .client {
    display: grid;
    gap: 12px;
    margin-top: 14px;
    padding-top: 14px;
    border-top: 1px solid var(--rim);
    max-width: 520px;
  }
  .docs {
    justify-self: start;
    padding: 0;
    height: auto;
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
</style>
