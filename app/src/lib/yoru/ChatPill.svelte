<!-- The last things said to Yoru and what it answered, newest at the bottom, like a chat. -->
<script lang="ts">
  import Pill from "./Pill.svelte";
  import { monitor } from "../monitor.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { settingsNav } from "../settings/nav.svelte";

  const t = $derived(i18n.t.yoru.chat);
  const recent = $derived(monitor.transcripts.slice(-3));
  const listening = $derived(monitor.voice === "listening" || monitor.voice === "thinking");
</script>

<Pill title={t.title} accent="#8f86ff" action={{ label: t.all, onclick: () => settingsNav.go({ tab: "voice", id: "transcripts" }) }}>
  {#if recent.length}
    <ol>
      {#each recent as e (e.id)}
        <li class="you">{e.transcript?.text?.trim() || t.unheard}</li>
        {#if e.reply?.text}<li class="yoru">{e.reply.text}</li>{/if}
      {/each}
      {#if listening}<li class="yoru typing" aria-hidden="true"><i></i><i></i><i></i></li>{/if}
    </ol>
  {:else}
    <p class="empty">{t.empty}</p>
  {/if}
</Pill>

<style>
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    max-width: 86%;
    padding: 8px 11px;
    border-radius: 14px;
    font: 450 12.5px/1.4 var(--sans);
    user-select: text;
    -webkit-user-select: text;
  }
  .you {
    align-self: flex-end;
    background: rgba(255, 255, 255, 0.08);
    border-bottom-right-radius: 5px;
  }
  .yoru {
    align-self: flex-start;
    background: color-mix(in srgb, #8f86ff 16%, transparent);
    color: #dcd8ff;
    border-bottom-left-radius: 5px;
  }
  .typing {
    display: flex;
    gap: 4px;
    padding: 11px 12px;
  }
  .typing i {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
    animation: dot 1s ease-in-out infinite;
  }
  .typing i:nth-child(2) {
    animation-delay: 160ms;
  }
  .typing i:nth-child(3) {
    animation-delay: 320ms;
  }
  .empty {
    margin: 0;
    font: 450 12.5px/1.45 var(--sans);
    color: var(--faint);
  }
  @keyframes dot {
    50% {
      opacity: 0.25;
      transform: translateY(-2px);
    }
  }
</style>
