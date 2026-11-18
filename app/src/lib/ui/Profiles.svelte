<!--
  The setups (profiles.ts): four tiles, each saying what it costs and what
  it sends where, and under the one in use only what it still needs (its
  key, the Ollama model, Gemini Live's voice). "Test Yoru" goes through the
  chain in order and says, in plain words, the first thing that doesn't work.
  Used by Settings' first card and by the first run.
-->
<script lang="ts">
  import { call, why } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { PROFILES, PROFILE, applyProfile, currentProfile, missingKeys, type ProfileId } from "../profiles";
  import { store } from "../store.svelte";
  import type { KeyCheck, KeySource, VoiceInfo } from "../types";
  import KeyField from "./KeyField.svelte";

  let { compact = false, onchange }: { compact?: boolean; onchange?: () => void } = $props();
  const t = $derived(i18n.t.profiles);
  const tb = $derived(i18n.t.board);
  let voice = $state<VoiceInfo | null>(null);
  let busy = $state<ProfileId | null>(null);
  let error = $state("");

  const read = async () => (voice = await call<VoiceInfo>("voice_info").catch(() => voice));
  $effect(() => {
    void read();
  });

  const current = $derived(currentProfile(store.info?.brain ?? null, voice));
  const missing = $derived(current && voice ? missingKeys(current, voice) : []);
  const NAMES: Record<string, string> = { anthropic: "Anthropic", openai: "OpenAI", google: "Google AI Studio", groq: "Groq", elevenlabs: "ElevenLabs" };
  const nameOf = (p: string) => NAMES[p] ?? p;
  const PLACEHOLDERS: Record<string, string> = { anthropic: "sk-ant-…", openai: "sk-proj-…", google: "AIza…", groq: "gsk_…" };
  function sourceOf(provider: string): KeySource | null {
    const i = store.info;
    if (provider === "anthropic") return i?.key ?? null;
    if (provider === "google") return i?.google_key ?? null;
    if (provider === "groq") return i?.groq_key ?? null;
    return voice?.providers.find((p) => p.id === provider)?.key ?? null;
  }

  async function pick(id: ProfileId) {
    if (!voice) return;
    busy = id;
    error = "";
    try {
      voice = await applyProfile(id, voice);
      onchange?.();
    } catch (e) {
      error = why(e);
    } finally {
      busy = null;
    }
  }

  async function keyed() {
    await store.refreshInfo();
    await read();
    // A key added for the best setup's voice: speak with it now.
    if (current === "best" && voice) voice = await applyProfile("best", voice).catch(() => voice);
    onchange?.();
  }

  const setLive = (name: string) => voice && call<VoiceInfo>("set_voice", { settings: { ...$state.snapshot(voice.settings), live: name } }).then((v) => (voice = v));

  // The test: brain, hearing, voice, then a sentence out loud.
  type Step = { label: string; state: "wait" | "ok" | "wrong"; text: string };
  let steps = $state<Step[]>([]);
  let testing = $state(false);
  async function keyWorks(provider: string): Promise<string | null> {
    const c = await call<KeyCheck>("check_api_key", { provider }).catch((e) => ({ state: "unknown", error: why(e) }) as KeyCheck);
    return c.state === "valid" ? null : c.state === "invalid" ? t.test.badKey(nameOf(provider)) : (c.error ?? t.test.unreachable);
  }
  async function step(label: string, check: () => Promise<string | null>): Promise<boolean> {
    const s: Step = { label, state: "wait", text: "" };
    steps.push(s);
    const at = steps.length - 1;
    const problem = await check().catch((e) => why(e));
    steps[at] = { label, state: problem ? "wrong" : "ok", text: problem ?? "" };
    return !problem;
  }
  async function test() {
    testing = true;
    steps = [];
    await read();
    const v = voice!;
    const brain = store.info?.brain ?? null;
    const ok =
      (await step(t.test.brain, async () => {
        if (!brain) return t.test.noBrain;
        if (!store.info?.ready) return brain === "ollama" ? t.test.noOllama : t.test.badKey(nameOf(brain));
        return brain === "ollama" ? null : keyWorks(brain);
      })) &&
      (await step(t.test.hear, async () => {
        if (v.settings.live) return keyWorks("google");
        if (!v.settings.enabled) return t.test.notListening;
        if (v.settings.model.includes(":")) return keyWorks(v.settings.model.slice(0, v.settings.model.indexOf(":")));
        if (v.stt === "error") return v.stt_error ?? t.test.sttError;
        const m = v.models.find((x) => x.id === v.settings.model);
        return m && !m.installed ? t.test.download : null;
      })) &&
      (await step(t.test.speak, async () => {
        if (v.settings.live) return null;
        if (!v.settings.speak) return t.test.silent;
        if (v.tts === "error") return v.tts_error ?? t.test.ttsError;
        return null;
      }));
    if (ok)
      await step(t.test.aloud, async () => {
        if (store.boardState === "off") return t.test.noBoard;
        await call("test_voice", { board: null, language: i18n.lang });
        return null;
      });
    testing = false;
  }
</script>

<section class="tiles profiles" aria-label={t.title}>
  {#each PROFILES as id (id)}
    <button class="tile" class:on={current === id} disabled={!!busy} onclick={() => pick(id)}>
      <b>{t.names[id]}</b>
      <span>{t.notes[id]}</span>
      {#if !compact}<span class="uses faint">{t.uses[id]}</span>{/if}
      {#if busy === id}<span class="tag">…</span>{:else if current === id}<span class="tag">{t.inUse}</span>{/if}
    </button>
  {/each}
</section>
{#if voice && !current}<p class="dim note">{t.custom}</p>{/if}

{#if current && voice}
  {#each missing as k (k)}
    <section class="panel">
      <div class="block">
        <div class="what"><b>{t.needsKey(k)}</b><span class="dim">{t.keyWhy[k] ?? ""}</span></div>
        <KeyField provider={k} source={sourceOf(k)} placeholder={PLACEHOLDERS[k]} onchange={keyed} />
      </div>
    </section>
  {/each}
  {#if current === "best"}
    <section class="panel">
      <div class="block">
        <div class="what"><b>{t.elevenOptional}</b><span class="dim">{t.elevenWhy}</span></div>
        <KeyField provider="elevenlabs" source={sourceOf("elevenlabs")} onchange={keyed} />
      </div>
    </section>
  {:else if current === "private"}
    <p class="dim note">{t.privateHint}</p>
  {:else if current === "live"}
    <section class="panel">
      <div class="setting">
        <div class="what"><b>{t.liveVoice}</b><span class="dim">{t.liveHint}</span></div>
        <div class="how">
          <select value={voice.settings.live} onchange={(e) => setLive(e.currentTarget.value)} aria-label={t.liveVoice}>
            {#each ["male", "female"] as kind (kind)}
              <optgroup label={tb.liveKinds[kind]}>
                {#each voice.live_voices.filter((v) => v[2] === kind) as [name, style] (name)}<option value={name}>{name} · {tb.liveStyles[style] ?? style}</option>{/each}
              </optgroup>
            {/each}
          </select>
        </div>
      </div>
    </section>
  {/if}
{/if}

{#if !compact}
  <div class="test">
    <button class="btn" disabled={testing} onclick={test}>{testing ? t.test.running : t.test.button}</button>
    <span class="dim small">{t.test.hint}</span>
  </div>
  {#if steps.length}
    <ol class="steps">
      {#each steps as s, i (i)}
        <li><span class="dot {s.state === 'ok' ? 'done' : s.state === 'wrong' ? 'wrong' : 'think'}"></span><b>{s.label}</b>{#if s.text}<span class="error-text">{s.text}</span>{/if}</li>
      {/each}
      {#if !testing && steps.every((s) => s.state === "ok")}<li class="dim">{t.test.allGood}</li>{/if}
    </ol>
  {/if}
{/if}
{#if error}<p class="error-text note">{error}</p>{/if}

<style>
  .profiles {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .block {
    display: grid;
    gap: 12px;
    padding: 6px 0 16px;
  }
  .block .what {
    display: grid;
    gap: 2px;
  }
  .block .what b {
    font-weight: 550;
  }
  .block .what span {
    font-size: var(--fs-meta);
  }
  .uses {
    font-size: var(--fs-small);
    margin-top: 2px;
  }
  .test {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 18px;
  }
  .steps {
    list-style: none;
    margin: 10px 0 0;
    padding: 0;
    display: grid;
    gap: 6px;
    font-size: var(--fs-body);
  }
  .steps li {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
  }
</style>
