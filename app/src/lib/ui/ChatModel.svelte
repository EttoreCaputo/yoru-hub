<!--
  The model Yoru answers with, picked from a conversation's head: every
  service with a key (and the Ollama models tried with Yoru, when it has
  any): only the models known to work. Picking one sets the brain
  and its model in the settings, as Settings → Conversation does; a new
  brain starts its messages afresh, reminded of the conversation's last lines.
-->
<script lang="ts">
  import { call } from "../api";
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import type { AssistantSettings, Brain } from "../types";
  import ModelPicker, { type Choice } from "./ModelPicker.svelte";

  const NAMES: Record<Brain, string> = { anthropic: "Claude", openai: "OpenAI", google: "Gemini", groq: "Groq", ollama: "Ollama" };
  const FIELD = { anthropic: "model", openai: "openai_model", google: "google_model", groq: "groq_model", ollama: "ollama_model" } as const;

  const info = $derived(store.info);
  const s = $derived(info?.settings);
  const brain = $derived(info?.brain ?? null);

  // Ollama's models, read once: it may not even be running.
  let ollama = $state<string[]>([]);
  $effect(() => {
    call<string[]>("ollama_models").then((m) => (ollama = m), () => {});
  });

  const keyed = $derived<Brain[]>(
    !info ? [] : (["anthropic", "openai", "google", "groq"] as Brain[]).filter((b) => (b === "anthropic" ? info.key : b === "openai" ? info.openai_key : b === "google" ? info.google_key : info.groq_key)),
  );

  const choices = $derived.by<Choice[]>(() => {
    if (!s || !info) return [];
    const settings = i18n.t.settings;
    const out: Choice[] = [];
    for (const b of keyed) {
      const described: [string, string][] =
        b === "anthropic"
          ? [
              ["claude-opus-5-5", settings.claudeModels.opus],
              ["claude-sonnet-5-5", settings.claudeModels.sonnet],
              ["claude-haiku-5-5", settings.claudeModels.haiku],
            ]
          : ((b === "openai" ? info.openai_models : b === "google" ? info.google_models : info.groq_models) ?? []).map((m) => [m.id, m.note]);
      for (const [model, note] of described) {
        const [name, ...rest] = note.split(b === "anthropic" ? " — " : ":");
        out.push({ value: `${b}:${model}`, label: `${NAMES[b]} ${name.trim() || model}`, note: rest.join(":").trim() || model, group: NAMES[b] });
      }
    }
    for (const m of ollama) out.push({ value: `ollama:${m}`, label: m, group: "Ollama" });
    // The one in use, even when no list has it (a Claude model typed in Settings).
    if (brain && s[FIELD[brain]] && !out.some((c) => c.value === value)) out.unshift({ value, label: s[FIELD[brain]], group: NAMES[brain] });
    return out;
  });

  const value = $derived(brain && s ? `${brain}:${s[FIELD[brain]]}` : "");

  function pick(v: string) {
    const at = v.indexOf(":");
    const b = (at > 0 ? v.slice(0, at) : brain) as Brain | null;
    if (!b || !(b in FIELD)) return;
    const model = at > 0 ? v.slice(at + 1) : v;
    void store.saveSettings((x: AssistantSettings) => {
      x.provider = b;
      x[FIELD[b]] = model;
    });
  }
</script>

{#if value}
  <ModelPicker {value} {choices} compact label={i18n.t.home.model} onchange={pick} />
{/if}
