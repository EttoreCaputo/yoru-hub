<!--
  What Yoru said with a cloud voice, each source on its own tab, never
  mixed: ElevenLabs a sentence at a time (the text it sent, audio tags and
  all), Gemini Live an answer at a time. The sound to play again. Read
  again each time you come back.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import type { SpokenClip } from "../types";
  import ConfirmButton from "../ui/ConfirmButton.svelte";
  import { clock } from "../ui/when";

  const t = $derived(i18n.t.spoken);
  let clips = $state<SpokenClip[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let playing = $state<string | null>(null);
  let filter = $state("");
  // A clip's source is its voice's prefix (core/src/spoken.rs).
  const SOURCES = ["elevenlabs", "gemini-live"] as const;
  type Source = (typeof SOURCES)[number];
  let tab = $state<Source>("elevenlabs");
  const sourceOf = (c: SpokenClip) => c.voice.split(":")[0];
  const mine = $derived(clips.filter((c) => sourceOf(c) === tab));
  const count = (s: Source) => clips.filter((c) => sourceOf(c) === s).length;
  // One sound at a time: playing another stops this one. Web Audio rather
  // than an <audio> on a blob URL: the context is resumed in the click
  // itself, so WebKit lets it play once the clip arrives.
  let context: AudioContext | null = null;
  let source: AudioBufferSourceNode | null = null;

  const list = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? mine.filter((c) => c.text.toLowerCase().includes(q)) : mine;
  });

  $effect(() => {
    call<SpokenClip[]>("spoken_clips").then(
      (c) => ((clips = c), (loaded = true)),
      (e) => (error = why(e)),
    );
    return () => {
      stop();
      void context?.close();
      context = null;
    };
  });

  function stop() {
    const node = source;
    source = null;
    playing = null;
    try {
      node?.stop();
    } catch {
      // Not started yet.
    }
  }

  async function play(id: string) {
    const again = playing === id;
    stop();
    if (again) return;
    error = "";
    context ??= new AudioContext();
    const ready = context.resume();
    playing = id;
    try {
      const wav = bytes(await call<ArrayBuffer | Uint8Array | number[]>("spoken_audio", { id }));
      await ready;
      if (playing !== id || !context) return;
      const node = context.createBufferSource();
      node.buffer = samples(context, wav);
      node.connect(context.destination);
      node.onended = () => {
        if (source === node) stop();
      };
      source = node;
      node.start();
    } catch (e) {
      if (playing === id) stop();
      error = why(e);
    }
  }

  /** The command's bytes: Tauri hands them over as an ArrayBuffer, a typed
   *  array or a JSON list of numbers, depending on the build. */
  function bytes(raw: ArrayBuffer | Uint8Array | number[]): ArrayBuffer {
    if (raw instanceof ArrayBuffer) return raw;
    const view = ArrayBuffer.isView(raw) ? raw : Uint8Array.from(raw);
    return view.buffer.slice(view.byteOffset, view.byteOffset + view.byteLength) as ArrayBuffer;
  }

  /** A 16-bit PCM WAV (what the clips are) as an audio buffer. */
  function samples(context: AudioContext, wav: ArrayBuffer): AudioBuffer {
    const view = new DataView(wav);
    const tag = (at: number) => String.fromCharCode(...new Uint8Array(wav, at, 4));
    if (wav.byteLength < 12 || tag(0) !== "RIFF" || tag(8) !== "WAVE") throw new Error(t.notWav);
    let channels = 1;
    let rate = 16000;
    for (let at = 12; at + 8 <= wav.byteLength; ) {
      const size = view.getUint32(at + 4, true);
      const body = at + 8;
      if (tag(at) === "fmt ") {
        channels = view.getUint16(body + 2, true);
        rate = view.getUint32(body + 4, true);
      } else if (tag(at) === "data") {
        const frames = Math.floor(Math.min(size, wav.byteLength - body) / 2 / channels);
        const buffer = context.createBuffer(channels, frames, rate);
        for (let ch = 0; ch < channels; ch++) {
          const out = buffer.getChannelData(ch);
          for (let i = 0; i < frames; i++) out[i] = view.getInt16(body + (i * channels + ch) * 2, true) / 32768;
        }
        return buffer;
      }
      at = body + size + (size & 1);
    }
    throw new Error(t.notWav);
  }

  async function remove(id: string) {
    if (playing === id) stop();
    try {
      clips = await call<SpokenClip[]>("delete_spoken", { id });
    } catch (e) {
      error = why(e);
    }
  }

  let exporting = $state(false);
  let exported = $state("");
  /** The clips in view (all, or those the search finds) into a zip in Downloads. */
  async function exportZip() {
    exporting = true;
    error = "";
    exported = "";
    try {
      const ids = filter.trim() ? list.map((c) => c.id) : [];
      exported = await call<string>("export_spoken", { source: tab, ids });
    } catch (e) {
      error = why(e);
    } finally {
      exporting = false;
    }
  }

  async function clear() {
    stop();
    try {
      await call("clear_spoken", { source: tab });
      clips = clips.filter((c) => sourceOf(c) !== tab);
    } catch (e) {
      error = why(e);
    }
  }

  const when = (at: number) => `${i18n.date(at)} · ${i18n.time(at)}`;
  // The audio tags, set apart from the words.
  const parts = (text: string) => text.split(/(\[[a-z ]+\])/i).filter(Boolean);
</script>

<div class="page spoken">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  <div class="seg sources" role="tablist">
    {#each SOURCES as s (s)}
      <button role="tab" aria-selected={tab === s} class:on={tab === s} onclick={() => ((tab = s), (exported = ""))}>
        {t.sources[s]}<span class="n">{count(s)}</span>
      </button>
    {/each}
  </div>
  <p class="faint small about">{t.about[tab]}</p>

  <div class="bar">
    <input type="text" placeholder={t.search} bind:value={filter} aria-label={t.search} />
    <span class="faint small">{t.count(mine.length)}</span>
    <span class="end">
      <button class="btn small" disabled={!list.length || exporting} onclick={exportZip}>{exporting ? t.exporting : filter.trim() ? t.exportFound(list.length) : t.export}</button>
      <ConfirmButton small label={t.clear} question={t.clearConfirm(t.sources[tab])} sure={t.clearSure} disabled={!mine.length} onconfirm={clear} />
    </span>
  </div>

  {#if error}<p class="error-text">{error}</p>{/if}
  {#if exported}<p class="faint small saved">{t.exported(exported)}</p>{/if}

  {#if list.length}
    <div class="list">
      {#each list as c (c.id)}
        <article class="clip" class:on={playing === c.id}>
          <button class="play" onclick={() => play(c.id)} aria-label={playing === c.id ? t.stop : t.play} title={playing === c.id ? t.stop : t.play}>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              {#if playing === c.id}<rect x="7" y="7" width="10" height="10" rx="1.5" />{:else}<path d="M8 5.5v13l10.5-6.5z" />{/if}
            </svg>
          </button>
          <div class="body">
            <header>
              <time class="faint" datetime={new Date(c.at).toISOString()}>{when(c.at)}</time>
              <span class="faint">{c.language.toUpperCase()} · {clock(c.duration_ms / 1000)}</span>
            </header>
            <p class="text">
              {#each parts(c.text) as p, i (i)}{#if /^\[.*\]$/.test(p)}<span class="tag">{p}</span>{:else}{p}{/if}{/each}
            </p>
          </div>
          <button class="btn small quiet" onclick={() => remove(c.id)}>{t.delete}</button>
        </article>
      {/each}
    </div>
  {:else if loaded}
    <div class="empty">
      <b>{mine.length ? t.noMatch : t.empty}</b>
      {#if !mine.length}<p>{t.emptyHint[tab]}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .spoken {
    max-width: 820px;
  }
  .small {
    font-size: 12.5px;
  }
  .sources {
    margin-bottom: 8px;
  }
  .sources .n {
    margin-left: 6px;
    opacity: 0.6;
  }
  .about {
    margin-bottom: 12px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--rim);
    margin-bottom: 4px;
  }
  .bar input {
    flex: 1;
    max-width: 320px;
  }
  .end {
    display: flex;
    gap: 8px;
    margin-left: auto;
  }
  .saved {
    margin: 8px 0 4px;
    user-select: text;
  }
  .clip {
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr) auto;
    gap: 12px;
    align-items: start;
    padding: 12px 4px;
  }
  .clip + .clip {
    border-top: 1px solid var(--rim);
  }
  .play {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 1px solid var(--rim-2);
    background: var(--ink-2);
    color: var(--pearl);
    cursor: pointer;
  }
  .play:hover,
  .on .play {
    background: var(--ink-3);
  }
  .play svg {
    width: 14px;
    height: 14px;
    fill: currentColor;
  }
  .body {
    min-width: 0;
  }
  header {
    display: flex;
    gap: 10px;
    font-size: 12px;
  }
  .text {
    margin-top: 3px;
    color: var(--pearl);
    font-size: 14px;
    line-height: 1.45;
    user-select: text;
  }
  .tag {
    color: var(--pearl-mid);
    font-style: italic;
  }
</style>
