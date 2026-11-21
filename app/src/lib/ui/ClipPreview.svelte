<!--
  One of Yoru's clips, drawn as the board draws it (lib/clips.ts): still on
  its most telling frame, played while `playing` (a looping clip goes round,
  another starts again after a short pause).
-->
<script lang="ts">
  import { drawFrame, type Clip } from "../clips";

  let { clip, playing = false, size = 132, rgb, mirror = false }: { clip: Clip; playing?: boolean; size?: number; rgb?: number; mirror?: boolean } = $props();

  /** The pause after a clip that ends, before it starts again. */
  const REST_MS = 700;

  let canvas: HTMLCanvasElement | undefined = $state();

  $effect(() => {
    const g = canvas?.getContext("2d");
    if (!g || !canvas) return;
    const ratio = window.devicePixelRatio || 1;
    canvas.width = canvas.height = Math.round(size * ratio);
    g.setTransform(ratio, 0, 0, ratio, 0, 0);
    const show = (i: number) => drawFrame(g, clip.frames[Math.min(i, clip.frames.length - 1)], size, rgb, mirror);
    if (!playing) {
      show(clip.still);
      return;
    }
    const frameMs = 1000 / clip.fps;
    const round = clip.frames.length * frameMs + (clip.loop ? 0 : REST_MS);
    const start = performance.now();
    let raf = 0;
    let last = -1;
    const tick = (now: number) => {
      const i = Math.floor(((now - start) % round) / frameMs);
      if (i !== last) show((last = i));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });
</script>

<canvas bind:this={canvas} style:width="{size}px" style:height="{size}px" aria-hidden="true"></canvas>

<style>
  canvas {
    display: block;
    border-radius: 50%;
  }
</style>
