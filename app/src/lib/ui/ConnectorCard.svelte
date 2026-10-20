<!--
  One connected service: its state, on or off, signing in and out, and its
  tools with what Yoru may do with each (use it; ask first because it acts).
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { ConnectorStatus, ToolInfo } from "../types";
  import ServiceIcon from "./ServiceIcon.svelte";
  import Toggle from "./Toggle.svelte";

  let { c, signingIn, loginError = "", onsignin }: { c: ConnectorStatus; signingIn: boolean; loginError?: string; onsignin: () => void } = $props();
  const t = $derived(i18n.t.connectors);
  let tools = $state(false);
  let error = $state("");
  let removing = $state(false);

  const dot = $derived(
    { connected: "done", connecting: "think", needs_login: "wait", error: "wrong", disabled: "" }[c.state] ?? "",
  );

  async function run(cmd: string, args: Record<string, unknown>) {
    error = "";
    try {
      await call(cmd, args);
    } catch (e) {
      error = why(e);
    } finally {
      await store.refreshConnectors();
    }
  }

  // Two clicks instead of confirm(): the webview may swallow native dialogs.
  const remove = () => (removing ? run("remove_connector", { id: c.id }) : (removing = true));
  // A failed sign-in can leave a half-saved registration behind even without tokens, so reset is offered whenever the service isn't plainly connected.
  const canReset = $derived(c.transport === "http" && (c.signed_in || c.state !== "connected"));
  const policy = (tool: ToolInfo, enabled: boolean, acts: boolean) => run("set_tool_policy", { id: c.id, tool: tool.name, enabled, acts });
</script>

<article class="panel conn" class:off={!c.enabled}>
  <div class="head">
    <ServiceIcon id={c.id} name={c.name} url={c.transport === "http" ? c.target : undefined} size={38} off={!c.enabled} />
    <div class="name">
      <h3>{c.name}</h3>
      <p class="mono faint target">{c.target}</p>
    </div>
    <div class="state"><span class="dot {dot}"></span>{signingIn ? t.signingIn : t.state[c.state]}</div>
    <Toggle checked={c.enabled} label={t.enabled} onchange={(on) => run("set_connector_enabled", { id: c.id, enabled: on })} />
  </div>

  {#if c.state === "error" && c.message}<p class="error-text msg">{c.message}</p>{/if}

  <div class="row actions">
    {#if signingIn}
      <button class="btn small" onclick={() => call("cancel_login", { id: c.id })}>{t.cancel}</button>
    {:else if c.enabled && c.transport === "http" && (c.state === "needs_login" || (c.state === "error" && !c.signed_in))}
      <button class="btn primary small" onclick={onsignin}>{t.signIn}</button>
    {/if}
    {#if c.tools.length}
      <button class="btn quiet small" aria-expanded={tools} onclick={() => (tools = !tools)}>{tools ? t.hideTools : `${t.showTools} · ${t.tools(c.tools.length)}`}</button>
    {/if}
    <span class="spacer"></span>
    {#if canReset}<button class="btn quiet small" onclick={() => run("logout_connector", { id: c.id })}>{c.signed_in ? t.signOut : t.resetSignIn}</button>{/if}
    {#if removing}
      <span class="faint sure">{t.removeConfirm(c.name)}</span>
      <button class="btn quiet small" onclick={() => (removing = false)}>{t.cancel}</button>
      <button class="btn small" onclick={remove}>{t.removeSure}</button>
    {:else}
      <button class="btn quiet small" onclick={remove}>{t.remove}</button>
    {/if}
  </div>
  {#if loginError && !signingIn}<p class="error-text">{loginError}</p>{/if}
  {#if error}<p class="error-text">{error}</p>{/if}

  {#if tools}
    <ul class="tools">
      {#each c.tools as tool (tool.name)}
        <li>
          <div class="what">
            <b class="mono">{tool.title ?? tool.name}</b>
            {#if tool.acts}<span class="tag acts" title={t.toolActsHint}>{t.toolActs}</span>{/if}
            {#if tool.description}<p class="dim">{tool.description}</p>{/if}
          </div>
          <label class="opt"><span>{t.toolOffered}</span><Toggle checked={tool.enabled} label={t.toolOffered} onchange={(on) => policy(tool, on, tool.acts)} /></label>
          <label class="opt" title={t.toolActsHint}><span>{t.toolActs}</span><Toggle checked={tool.acts} label={t.toolActs} disabled={!tool.enabled} onchange={(on) => policy(tool, tool.enabled, on)} /></label>
        </li>
      {/each}
    </ul>
  {:else if !c.tools.length && c.state !== "connected"}
    <p class="faint hint">{t.noTools}</p>
  {/if}
</article>

<style>
  .conn.off {
    opacity: 0.7;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .name {
    flex: 1;
    min-width: 0;
  }
  h3 {
    font: 600 15px/1.2 var(--sans);
  }
  .target {
    margin-top: 3px;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--haze);
    font-size: 12.5px;
    white-space: nowrap;
  }
  .msg {
    margin-top: 10px;
  }
  .actions {
    margin-top: 12px;
    gap: 4px;
  }
  .sure {
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .hint {
    margin-top: 8px;
    font-size: 12.5px;
  }
  .tools {
    list-style: none;
    margin: 12px 0 0;
    padding: 0;
    border-top: 1px solid var(--rim);
  }
  .tools li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: 18px;
    align-items: center;
    padding: 10px 0;
  }
  .tools li + li {
    border-top: 1px solid var(--rim);
  }
  .what b {
    font-weight: 600;
    font-size: 12.5px;
    margin-right: 6px;
  }
  .what p {
    margin-top: 2px;
    font-size: 12.5px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--haze);
    white-space: nowrap;
  }
</style>
