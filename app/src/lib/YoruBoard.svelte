<!--
  A Yoru, mirrored: one round 466 × 466 AMOLED with the little night
  creature of yoru/docs/CONCEPT.md in the black. A matte sphere, two dark
  pill eyes that slide towards the pointer, a tint in its lower half for
  what the voice is doing, and a halo of the same colour. With no board it
  drifts off into a dim dot, as Yoru does when it falls asleep.
-->
<script lang="ts">
  import type { BoardState, Gaze, VoiceState } from "./monitor.svelte";

  let {
    board,
    size,
    voice = "idle",
    gaze = null,
  }: {
    board: BoardState;
    size: number;
    voice?: VoiceState;
    gaze?: Gaze | null;
  } = $props();

  const D = 466;
  const C = D / 2;
  /** About 40% of the disc, as on the board. */
  const R = 93;

  /** Blue while thinking, green while answering, red on an error (CONCEPT.md, "State tint"). */
  const TINTS: Partial<Record<VoiceState, string>> = { thinking: "#4aa3ff", speaking: "#40e080", error: "#ff4d4d" };
  const tint = $derived(TINTS[voice] ?? null);
  const asleep = $derived(board === "off");
  const listening = $derived(voice === "listening");

  // The eyes slide over the surface towards the pointer, and narrow near the edge.
  const look = $derived(gaze && !asleep ? { x: Math.max(-1, Math.min(1, gaze.x)), y: Math.max(-1, Math.min(1, gaze.y)) } : { x: 0, y: 0 });
  const eyeX = $derived(look.x * R * 0.38);
  const eyeY = $derived(R * 0.12 + look.y * R * 0.22);
  const narrow = $derived(1 - Math.abs(look.x) * 0.3);
  const eyeH = $derived(asleep ? 4 : listening ? 40 : 32);
</script>

<div class="yoru" class:asleep style:width="{size * 1.15}px" style:height="{size * 1.15}px">
  <svg viewBox="0 0 {D} {D}" role="img" aria-label="Yoru">
    <defs>
      <radialGradient id="yoru-body" cx="0.36" cy="0.32" r="0.75">
        <stop offset="0" stop-color="#e9e6f6" />
        <stop offset="0.55" stop-color="#bdb7d8" />
        <stop offset="1" stop-color="#6e6890" />
      </radialGradient>
      <linearGradient id="yoru-tint" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0.45" stop-color={tint ?? "transparent"} stop-opacity="0" />
        <stop offset="1" stop-color={tint ?? "transparent"} stop-opacity="0.75" />
      </linearGradient>
      <radialGradient id="yoru-halo">
        <stop offset="0.4" stop-color={tint ?? "#000"} stop-opacity="0.35" />
        <stop offset="1" stop-color={tint ?? "#000"} stop-opacity="0" />
      </radialGradient>
      <clipPath id="yoru-sphere"><circle cx={C} cy={C} r={R} /></clipPath>
    </defs>

    <!-- The case, then the screen: true black, so the edge disappears. -->
    <circle cx={C} cy={C} r={C - 1} fill="#050506" stroke="rgba(255,255,255,.07)" stroke-width="2" />

    {#if tint && !asleep}<circle class="halo" cx={C} cy={C} r={R * 2} fill="url(#yoru-halo)" />{/if}

    <g class="body">
      <circle cx={C} cy={C} r={R} fill="url(#yoru-body)" />
      <rect class="tint" x={C - R} y={C - R} width={R * 2} height={R * 2} fill="url(#yoru-tint)" clip-path="url(#yoru-sphere)" opacity={tint ? 1 : 0} />
      <!-- A soft shadow towards the bottom edge, never a gloss. -->
      <circle cx={C} cy={C + R * 0.55} r={R * 0.9} fill="#000" opacity="0.12" clip-path="url(#yoru-sphere)" />
      <g class="eyes" style:transform="translate({eyeX}px, {eyeY}px) scaleX({narrow})">
        {#each [-1, 1] as side (side)}
          <rect class="eye" x={C + side * 22 - 7} y={C - eyeH / 2} width="14" height={eyeH} rx="7" fill="#17151f" />
        {/each}
      </g>
    </g>
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
    transform-origin: 50% 50%;
    transform-box: view-box;
    transition:
      transform 1200ms cubic-bezier(0.2, 0.8, 0.2, 1),
      opacity 1200ms ease;
    animation: bob 5s ease-in-out infinite;
  }
  .asleep .body {
    transform: scale(0.28);
    opacity: 0.35;
    animation: none;
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
  .asleep .eye {
    animation: none;
  }
  .tint,
  .halo {
    transition: opacity 600ms ease;
  }
  .halo {
    animation: glow 2.4s ease-in-out infinite;
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
  @keyframes glow {
    50% {
      opacity: 0.6;
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
