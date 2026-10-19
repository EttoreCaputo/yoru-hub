<!--
  A Yoru, mirrored: one round 466 × 466 AMOLED with the little night
  creature of yoru/docs/CONCEPT.md in the black. A matte sphere, two dark
  pill eyes that slide towards the pointer, two floating hands that pop out
  for a gesture, a tint in its lower half and a halo for what the voice is
  doing, and a badge for what Claude Code is doing. With no board it drifts
  off into a dim dot, as Yoru does when it falls asleep.

  The mirror only plays what it is given: `reaction` is a name from
  `YORU_REACTIONS`, and `at` restarts it when the same one comes again.
-->
<script lang="ts">
  import type { BoardState, Gaze, VoiceState } from "./types";

  export type Badge = "work" | "done" | "wait" | "error";

  let {
    board,
    size,
    voice = "idle",
    gaze = null,
    badge = null,
    dancing = false,
    reaction = null,
    onpoke,
  }: {
    board: BoardState;
    size: number;
    voice?: VoiceState;
    gaze?: Gaze | null;
    badge?: Badge | null;
    dancing?: boolean;
    reaction?: { name: string; at: number } | null;
    onpoke?: () => void;
  } = $props();

  const uid = $props.id();
  const D = 466;
  const C = D / 2;
  /** About 40% of the disc, as on the board. */
  const R = 93;

  /** Blue while thinking, green while answering, red on an error (CONCEPT.md, "State tint"). */
  const TINTS: Partial<Record<VoiceState, string>> = { thinking: "#4aa3ff", speaking: "#40e080", error: "#ff4d4d" };
  const BADGES: Record<Badge, string> = { work: "#4aa3ff", done: "#40e080", wait: "#ff9f43", error: "#ff4d4d" };
  const tint = $derived(TINTS[voice] ?? null);
  const asleep = $derived(board === "off");
  const listening = $derived(voice === "listening");

  // Which gesture the hands make, and which eyes, for a reaction (the board's `react` names).
  const GESTURES: Record<string, string> = {
    wave: "wave",
    clap: "clap",
    proud: "clap",
    excited: "clap",
    happy: "clap",
    shy: "cover",
    scared: "cover",
    stretch: "stretch",
    yawn: "stretch",
    point: "point",
    curious: "point",
    thinking: "think",
    dance: "sway",
    love: "hold",
  };
  const HAPPY_EYES = new Set(["happy", "excited", "proud", "love", "relieved", "charged", "dance", "clap", "wink"]);
  const CLOSED_EYES = new Set(["sleepy", "yawn", "tired", "drowsy", "sigh", "blink"]);
  const WIDE_EYES = new Set(["surprised", "scared", "falling", "lifted"]);

  /** The reaction playing now, cleared when it has run its course. */
  let playing = $state<string | null>(null);
  $effect(() => {
    if (!reaction || asleep) return;
    playing = reaction.name;
    const id = setTimeout(() => (playing = null), 1800);
    return () => clearTimeout(id);
  });
  const gesture = $derived(playing ? (GESTURES[playing] ?? null) : dancing ? "sway" : null);
  const eyes = $derived(
    asleep ? "closed" : !playing ? (listening ? "wide" : "open") : HAPPY_EYES.has(playing) ? "happy" : CLOSED_EYES.has(playing) ? "closed" : WIDE_EYES.has(playing) ? "wide" : "open",
  );

  // The eyes slide over the surface towards the pointer, and narrow near the edge.
  const look = $derived(gaze && !asleep && gesture !== "cover" ? { x: Math.max(-1, Math.min(1, gaze.x)), y: Math.max(-1, Math.min(1, gaze.y)) } : { x: 0, y: 0 });
  const eyeX = $derived(look.x * R * 0.38);
  const eyeY = $derived(R * 0.12 + look.y * R * 0.22 + (gesture === "think" ? -R * 0.18 : 0));
  const narrow = $derived(1 - Math.abs(look.x) * 0.3);
  const eyeH = $derived(eyes === "closed" ? 4 : eyes === "wide" ? 40 : 32);
</script>

<div class="yoru" class:asleep class:dancing={gesture === "sway"} style:width="{size * 1.15}px" style:height="{size * 1.15}px">
  <svg viewBox="0 0 {D} {D}" role="img" aria-label="Yoru">
    <defs>
      <radialGradient id="{uid}-body" cx="0.36" cy="0.32" r="0.75">
        <stop offset="0" stop-color="#eeebf8" />
        <stop offset="0.55" stop-color="#c3bedb" />
        <stop offset="1" stop-color="#6e6890" />
      </radialGradient>
      <linearGradient id="{uid}-tint" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0.45" stop-color={tint ?? "transparent"} stop-opacity="0" />
        <stop offset="1" stop-color={tint ?? "transparent"} stop-opacity="0.75" />
      </linearGradient>
      <radialGradient id="{uid}-halo">
        <stop offset="0.4" stop-color={tint ?? "#000"} stop-opacity="0.35" />
        <stop offset="1" stop-color={tint ?? "#000"} stop-opacity="0" />
      </radialGradient>
      <clipPath id="{uid}-sphere"><circle cx={C} cy={C} r={R} /></clipPath>
    </defs>

    <!-- The case, then the screen: true black, so the edge disappears. -->
    <circle cx={C} cy={C} r={C - 1} fill="#050506" stroke="rgba(255,255,255,.07)" stroke-width="2" />

    {#if tint && !asleep}<circle class="halo" cx={C} cy={C} r={R * 2} fill="url(#{uid}-halo)" />{/if}

    <!-- A poke squashes it: the button is the sphere itself. -->
    <g
      class="body"
      role="button"
      tabindex="-1"
      aria-label="Poke Yoru"
      onclick={() => onpoke?.()}
      onkeydown={(e) => e.key === "Enter" && onpoke?.()}
    >
      {#key reaction?.at}
        <g class="squash" class:poked={!!playing}>
          <circle cx={C} cy={C} r={R} fill="url(#{uid}-body)" />
          <rect class="tint" x={C - R} y={C - R} width={R * 2} height={R * 2} fill="url(#{uid}-tint)" clip-path="url(#{uid}-sphere)" opacity={tint ? 1 : 0} />
          <!-- A soft shadow towards the bottom edge, never a gloss. -->
          <circle cx={C} cy={C + R * 0.55} r={R * 0.9} fill="#000" opacity="0.12" clip-path="url(#{uid}-sphere)" />
          <g class="eyes" style:transform="translate({eyeX}px, {eyeY}px) scaleX({narrow})">
            {#each [-1, 1] as side (side)}
              {#if eyes === "happy"}
                <path class="arc" d="M {C + side * 22 - 9} {C + 4} q 9 -14 18 0" fill="none" stroke="#17151f" stroke-width="7" stroke-linecap="round" />
              {:else}
                <rect class="eye" class:still={eyes === "closed"} x={C + side * 22 - 7} y={C - eyeH / 2} width="14" height={eyeH} rx="7" fill="#17151f" />
              {/if}
            {/each}
          </g>
        </g>

        <!-- The hands: tucked behind the body until a gesture needs them. -->
        {#each [-1, 1] as side (side)}
          <g class="hand {gesture ?? 'rest'} {side < 0 ? 'left' : 'right'}">
            <ellipse cx={C + side * R * 0.62} cy={C + R * 0.25} rx="20" ry="15" fill="url(#{uid}-body)" />
          </g>
        {/each}
      {/key}
    </g>

    {#if badge && !asleep}
      <g class="badge" class:work={badge === "work"} style:--badge={BADGES[badge]}>
        <circle cx={C - R * 0.72} cy={C - R * 0.72} r="15" fill={BADGES[badge]} stroke="#050506" stroke-width="4" />
        {#if badge === "work"}
          {#each [-5.5, 0, 5.5] as dx, i (i)}
            <circle class="dot" style:animation-delay="{i * 160}ms" cx={C - R * 0.72 + dx} cy={C - R * 0.72} r="2.1" fill="#fff" />
          {/each}
        {:else if badge === "done"}
          <path d="M {C - R * 0.72 - 6} {C - R * 0.72} l 4 4 l 8 -8" fill="none" stroke="#06140b" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
        {:else}
          <rect x={C - R * 0.72 - 1.6} y={C - R * 0.72 - 7} width="3.2" height="9" rx="1.6" fill="#140805" />
          <circle cx={C - R * 0.72} cy={C - R * 0.72 + 6} r="1.9" fill="#140805" />
        {/if}
      </g>
    {/if}
    {#if asleep}<text class="z" x={C + 26} y={C - 40}>z</text>{/if}
  </svg>
</div>

<style>
  .yoru {
    display: grid;
    place-items: center;
  }
  svg {
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  .body {
    cursor: pointer;
    outline: none;
    transform-origin: 50% 50%;
    transform-box: view-box;
    transition:
      transform 1200ms cubic-bezier(0.2, 0.8, 0.2, 1),
      opacity 1200ms ease;
    animation: bob 5s ease-in-out infinite;
  }
  .dancing .body {
    animation: beat 0.5s ease-in-out infinite;
  }
  .asleep .body {
    transform: scale(0.28);
    opacity: 0.35;
    animation: none;
    cursor: default;
  }
  .squash {
    transform-box: view-box;
    transform-origin: 50% 70%;
  }
  .squash.poked {
    animation: poke 520ms cubic-bezier(0.3, 1.6, 0.4, 1);
  }
  .eyes {
    transform-box: view-box;
    transform-origin: 50% 50%;
    transition: transform 180ms ease-out;
  }
  .eye {
    transform-box: fill-box;
    transform-origin: 50% 50%;
    transition:
      height 200ms,
      y 200ms;
    animation: blink 4.2s infinite;
  }
  .eye.still,
  .asleep .eye {
    animation: none;
  }

  /* Hands: each gesture is a short path, as on the board (CONCEPT.md, "How it expresses itself"). */
  .hand {
    transform-box: view-box;
    transform-origin: 50% 50%;
    transition:
      transform 380ms cubic-bezier(0.3, 1.4, 0.4, 1),
      opacity 220ms ease;
  }
  /* Each hand moves away from the body on its own side. */
  .hand.left {
    --dir: 1;
  }
  .hand.right {
    --dir: -1;
  }
  .hand.rest {
    opacity: 0;
    transform: scale(0.7);
  }
  .hand.left.wave {
    transform: translate(-36px, 14px);
  }
  .hand.right.wave {
    animation: wave 1.6s ease-in-out both;
  }
  .hand.clap {
    animation: clap 1.6s ease-in-out both;
  }
  .hand.cover {
    animation: cover 1.8s ease-in-out both;
  }
  .hand.right.cover {
    animation-name: peek;
  }
  .hand.stretch {
    animation: stretch 1.8s ease-in-out both;
  }
  .hand.left.point {
    transform: translate(-30px, 10px);
  }
  .hand.right.point {
    transform: translate(64px, -22px) scaleX(1.25);
  }
  .hand.left.think {
    opacity: 0;
  }
  .hand.right.think {
    transform: translate(-42px, 54px);
  }
  .hand.hold {
    transform: translate(calc(var(--dir) * 22px), -6px);
  }
  .hand.sway {
    animation: sway 1s ease-in-out infinite;
  }
  .hand.right.sway {
    animation-delay: -0.5s;
  }

  .arc {
    animation: none;
  }
  .tint,
  .halo {
    transition: opacity 600ms ease;
  }
  .halo {
    animation: glow 2.4s ease-in-out infinite;
  }
  .badge {
    animation: pop 380ms cubic-bezier(0.3, 1.6, 0.4, 1) both;
    transform-box: fill-box;
    transform-origin: 50% 50%;
    filter: drop-shadow(0 0 10px var(--badge));
  }
  .dot {
    animation: dots 1.1s ease-in-out infinite;
  }
  .z {
    fill: rgba(255, 255, 255, 0.35);
    font: 600 22px/1 var(--sans);
    animation: drift 3s ease-in-out infinite;
  }
  @keyframes bob {
    50% {
      transform: translateY(-4px);
    }
  }
  @keyframes beat {
    50% {
      transform: translateY(-7px) scale(1.02, 0.98);
    }
  }
  @keyframes poke {
    30% {
      transform: scale(1.12, 0.86);
    }
    60% {
      transform: scale(0.95, 1.06);
    }
  }
  @keyframes blink {
    0%,
    95%,
    100% {
      transform: scaleY(1);
    }
    97% {
      transform: scaleY(0.08);
    }
  }
  @keyframes wave {
    0% {
      opacity: 0;
      transform: translate(0, 0) scale(0.7);
    }
    18% {
      opacity: 1;
      transform: translate(60px, -70px) rotate(0deg);
    }
    34%,
    66% {
      transform: translate(68px, -78px);
    }
    50%,
    82% {
      transform: translate(52px, -74px);
    }
    100% {
      opacity: 1;
      transform: translate(60px, -70px);
    }
  }
  @keyframes clap {
    0% {
      opacity: 0;
      transform: scale(0.7);
    }
    20% {
      opacity: 1;
      transform: translate(calc(var(--dir) * 20px), 40px);
    }
    35%,
    65% {
      transform: translate(calc(var(--dir) * 40px), 38px);
    }
    50%,
    80% {
      transform: translate(calc(var(--dir) * 48px), 36px);
    }
    100% {
      opacity: 1;
      transform: translate(calc(var(--dir) * 40px), 38px);
    }
  }
  @keyframes cover {
    0% {
      opacity: 0;
      transform: scale(0.7);
    }
    25%,
    60% {
      opacity: 1;
      transform: translate(calc(var(--dir) * 36px), -10px);
    }
    100% {
      opacity: 1;
      transform: translate(calc(var(--dir) * 36px), -10px);
    }
  }
  /* Then one hand slides down, to peek. */
  @keyframes peek {
    0% {
      opacity: 0;
      transform: scale(0.7);
    }
    25%,
    60% {
      opacity: 1;
      transform: translate(-36px, -10px);
    }
    100% {
      opacity: 1;
      transform: translate(-30px, 34px);
    }
  }
  @keyframes stretch {
    0% {
      opacity: 0;
      transform: scale(0.7);
    }
    40%,
    75% {
      opacity: 1;
      transform: translate(calc(var(--dir) * -62px), -30px);
    }
    100% {
      opacity: 1;
      transform: translate(calc(var(--dir) * -40px), 0);
    }
  }
  @keyframes sway {
    0%,
    100% {
      opacity: 1;
      transform: translate(calc(var(--dir) * -62px), 6px);
    }
    50% {
      opacity: 1;
      transform: translate(calc(var(--dir) * -62px), -12px);
    }
  }
  @keyframes glow {
    50% {
      opacity: 0.6;
    }
  }
  @keyframes pop {
    from {
      transform: scale(0);
    }
  }
  @keyframes dots {
    50% {
      opacity: 0.25;
    }
  }
  @keyframes drift {
    0% {
      opacity: 0;
      transform: translateY(6px);
    }
    50% {
      opacity: 1;
    }
    100% {
      opacity: 0;
      transform: translateY(-14px);
    }
  }
</style>
