<!--
  Yoru's home in the window. Not a dashboard: Yoru never shows gauges
  (yoru/docs/CONCEPT.md), so the computer's state is told the way coucou's
  island does, one line at a time over the pet, and the details sit in
  pills beside it: Claude Code, music, focus, conversations and mood.
-->
<script lang="ts">
  import YoruBoard from "../YoruBoard.svelte";
  import ClaudePill from "./ClaudePill.svelte";
  import AgentsPill from "./AgentsPill.svelte";
  import MusicPill from "./MusicPill.svelte";
  import TimerPill from "./TimerPill.svelte";
  import ChatPill from "./ChatPill.svelte";
  import MoodPill from "./MoodPill.svelte";
  import { activity, type Tone } from "./activity.svelte";
  import { monitor } from "../monitor.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { settingsNav } from "../settings/nav.svelte";

  let { winH }: { winH: number } = $props();

  const t = $derived(i18n.t.yoru);
  const board = $derived(monitor.boardState);
  const off = $derived(board === "off");
  let stageW = $state(0);
  const size = $derived(Math.round(Math.max(170, Math.min(330, stageW * 0.62, winH * 0.42))));

  $effect(() => activity.observeClaude(monitor.last?.claude?.state));

  const TONES: Record<Tone, string> = {
    listen: "#8f86ff",
    work: "#4aa3ff",
    done: "#40e080",
    wait: "#ff9f43",
    music: "#40e080",
    calm: "#9a9ea8",
    off: "#5d616a",
  };

  /** The one thing worth saying now, most pressing first. */
  const island = $derived.by((): { tone: Tone; text: string; detail?: string } => {
    if (off) return { tone: "off", text: t.island.asleep };
    const v = monitor.voice;
    if (v === "listening") return { tone: "listen", text: t.island.listening };
    if (v === "thinking") return { tone: "listen", text: t.island.thinking };
    if (v === "speaking") return { tone: "listen", text: t.island.speaking, detail: monitor.transcripts.at(-1)?.reply?.text };
    const s = monitor.last;
    if (s?.timer?.state === "ring") return { tone: "wait", text: t.island.ring(s.timer.label) };
    const asks = monitor.agents.approvals[0];
    if (asks) return { tone: "wait", text: t.island.claudeAsks, detail: asks.project ? `${asks.project} · ${asks.text}` : asks.text };
    const waits = monitor.agents.sessions.find((a) => a.state === "wait");
    if (waits) return { tone: "wait", text: t.island.claudeAsks, detail: waits.project };
    if (s?.claude?.state === "work") return { tone: "work", text: t.island.claudeWork, detail: s.claude.model };
    if (activity.claudeDone) return { tone: "done", text: t.island.claudeDone };
    if (s?.music?.state === "play") return { tone: "music", text: s.music.title, detail: s.music.artist };
    if (s?.timer?.state === "run") return { tone: "wait", text: s.timer.label ?? t.timer.kinds[s.timer.kind], detail: t.island.timer(s.timer.left_s) };
    const pet = monitor.pet;
    if (pet) return { tone: "calm", text: t.island.mood((i18n.t.pet.moods[pet.mood] ?? i18n.t.pet.moods.content).label) };
    return { tone: "calm", text: t.island.hello };
  });

  const REACTIONS = [
    ["wave", "👋"],
    ["happy", "😊"],
    ["dance", "🎵"],
    ["love", "💗"],
    ["curious", "👀"],
    ["surprised", "😮"],
    ["sleepy", "😴"],
    ["dizzy", "💫"],
  ] as const;

  // A wave hello when it comes online, as the board does when you come back.
  let greeted = false;
  $effect(() => {
    if (board === "live" && !greeted) {
      greeted = true;
      activity.reaction = { name: "wave", at: Date.now() };
    }
    if (off) greeted = false;
  });
</script>

<section class="home">
  <div class="stage" bind:clientWidth={stageW} data-tour="board">
    <div class="island tone-{island.tone}" style:--tone={TONES[island.tone]}>
      <span class="mini" aria-hidden="true"><i></i><i></i></span>
      {#key island.text}
        <span class="text">{island.text}</span>
      {/key}
      {#if island.detail}<span class="detail">{island.detail}</span>{/if}
      {#if island.tone === "work" || island.tone === "listen"}<span class="wave" aria-hidden="true"><b></b><b></b><b></b></span>{/if}
    </div>

    <div class="pet" class:off title={off ? "" : t.poke}>
      <YoruBoard
        {board}
        {size}
        voice={monitor.voice}
        gaze={monitor.gaze}
        badge={activity.badge}
        dancing={monitor.last?.music?.state === "play"}
        reaction={activity.reaction}
        onpoke={() => !off && activity.poke()}
      />
      <div class="floor" aria-hidden="true"></div>
    </div>

    {#if off}
      <div class="offline">
        <b>{t.offline.title}</b>
        <span>{t.offline.text}</span>
        <button onclick={() => settingsNav.go({ tab: "device", id: "connection" })}>{t.offline.connection}</button>
      </div>
    {:else}
      <div class="reactions" role="group" aria-label={t.react.label}>
        {#each REACTIONS as [name, glyph] (name)}
          <button title={t.react[name]} onclick={() => activity.react(name)}>
            <span aria-hidden="true">{glyph}</span><span class="name">{t.react[name]}</span>
          </button>
        {/each}
      </div>
    {/if}
    <div class="spec" aria-hidden="true">{t.spec}</div>
  </div>

  <div class="pills" data-tour="telemetry">
    <ClaudePill />
    <AgentsPill />
    <MusicPill />
    <TimerPill />
    <ChatPill />
    <MoodPill />
  </div>
</section>

<style>
  .home {
    position: relative;
    grid-row: 3 / span 2;
    display: grid;
    grid-template-columns: minmax(360px, 1.15fr) minmax(300px, 1fr);
    gap: 18px;
    padding: 6px 18px 18px;
    min-height: 0;
  }

  .stage {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
    min-height: 0;
    padding: 58px 0 16px;
    border-radius: 28px;
    background:
      radial-gradient(55% 45% at 50% 46%, rgba(120, 110, 255, 0.07), transparent 70%),
      radial-gradient(80% 60% at 50% 100%, rgba(0, 0, 0, 0.5), transparent 70%);
    contain: paint;
  }

  /* The island: a black capsule over the pet, like coucou's notch. */
  .island {
    position: absolute;
    z-index: 2;
    top: 14px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 9px;
    max-width: calc(100% - 40px);
    height: 34px;
    padding: 0 14px 0 8px;
    border-radius: 999px;
    background: #000;
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow:
      0 10px 30px -12px rgba(0, 0, 0, 0.9),
      0 0 0 4px color-mix(in srgb, var(--tone) 7%, transparent);
    transition: box-shadow 400ms;
    white-space: nowrap;
  }
  .mini {
    flex: none;
    position: relative;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: radial-gradient(circle at 36% 30%, #f1eef9, #c3bedb 55%, #6e6890);
    box-shadow: 0 0 10px -2px var(--tone);
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 3px;
    padding-top: 3px;
  }
  .mini i {
    width: 2.6px;
    height: 5.5px;
    border-radius: 2px;
    background: #17151f;
  }
  .text {
    font: 550 12.5px/1 var(--sans);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    animation: slide 320ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .detail {
    font: 450 12px/1 var(--sans);
    color: color-mix(in srgb, var(--tone) 55%, var(--dim));
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .wave {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 12px;
  }
  .wave b {
    width: 2.5px;
    height: 100%;
    border-radius: 2px;
    background: var(--tone);
    animation: eq 0.9s ease-in-out infinite;
  }
  .wave b:nth-child(2) {
    animation-delay: -0.3s;
  }
  .wave b:nth-child(3) {
    animation-delay: -0.6s;
  }

  .pet {
    position: relative;
    display: grid;
    place-items: center;
    transition:
      filter 800ms ease,
      opacity 800ms ease;
    animation: rise 1100ms cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }
  .pet.off {
    filter: saturate(0.6) brightness(0.85);
  }
  .floor {
    position: absolute;
    bottom: 2%;
    width: 46%;
    height: 14px;
    border-radius: 50%;
    background: radial-gradient(closest-side, rgba(0, 0, 0, 0.7), transparent);
    z-index: -1;
  }

  .reactions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    max-width: 440px;
    padding: 0 16px;
  }
  .reactions button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 11px 0 9px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.03);
    color: var(--dim);
    font: 500 12px/1 var(--sans);
    cursor: pointer;
    transition:
      background 160ms,
      color 160ms,
      transform 160ms;
  }
  .reactions button:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
    transform: translateY(-1px);
  }
  .reactions button:active {
    transform: scale(0.96);
  }

  .offline {
    display: grid;
    justify-items: center;
    gap: 6px;
    text-align: center;
    font: 450 12.5px/1.4 var(--sans);
    color: var(--dim);
  }
  .offline b {
    font: 600 14px/1.2 var(--sans);
    color: var(--text);
  }
  .offline button {
    margin-top: 6px;
    padding: 7px 14px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.05);
    color: var(--text);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
  }

  .spec {
    margin-top: 4px;
    font: 500 9.5px/1 var(--mono);
    letter-spacing: 0.12em;
    color: var(--faint);
    opacity: 0.7;
  }

  .pills {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(270px, 1fr));
    align-content: start;
    gap: 12px;
    min-height: 0;
    overflow-y: auto;
    padding: 2px 2px 8px;
  }

  /* One column, and the window scrolls instead of squeezing the stage. */
  @media (max-width: 860px) {
    .home,
    .stage {
      min-height: auto;
    }
    .home {
      grid-template-columns: 1fr;
    }
    .pills {
      overflow: visible;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(24px) scale(0.97);
    }
  }
  @keyframes slide {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  @keyframes eq {
    0%,
    100% {
      transform: scaleY(0.35);
    }
    50% {
      transform: scaleY(1);
    }
  }
</style>
