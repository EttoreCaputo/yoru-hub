/**
 * The setups: one choice that sets how Yoru thinks, hears and speaks, so
 * nobody has to know what a brain, a transcription or a voice service is
 * to get him talking. Each one is a starting point: the brain and voice
 * pages still change any part of it, and then the setup reads "Custom".
 */
import { call } from "./api";
import { store } from "./store.svelte";
import type { AccountVoice, AssistantSettings, Brain, VoiceInfo, VoiceSettings } from "./types";

export type ProfileId = "free" | "private" | "best" | "live";
export const PROFILES: ProfileId[] = ["free", "best", "live", "private"];

/** What each setup uses, and the keys it can't do without. */
export const PROFILE: Record<ProfileId, { brain: Brain; keys: string[]; hear: string; speak: string }> = {
  free: { brain: "groq", keys: ["groq"], hear: "groq", speak: "local" },
  best: { brain: "anthropic", keys: ["anthropic"], hear: "local", speak: "elevenlabs" },
  live: { brain: "google", keys: ["google"], hear: "google", speak: "google" },
  private: { brain: "ollama", keys: [], hear: "local", speak: "local" },
};

/** Gemini Live's voice when the live setup turns it on: a male one, as Yoru is. */
const LIVE_VOICE = "Puck";
const GROQ_WHISPER = "groq:whisper-large-v3-turbo";
const LOCAL_VOICES: Record<string, string> = { it: "it_IT-paola-medium", en: "en_GB-alba-medium" };

const local = (id: string | undefined) => !!id && !id.includes(":");

/** The setup the settings are in now, or null: someone picked the parts themselves. */
export function currentProfile(brain: Brain | null, voice: VoiceInfo | null): ProfileId | null {
  const v = voice?.settings;
  if (!brain || !v) return null;
  if (v.live) return brain === "google" ? "live" : null;
  const hears = !v.enabled ? null : local(v.model) ? "local" : v.model.slice(0, v.model.indexOf(":"));
  const voices = Object.values(v.voices);
  const speaks = !v.speak ? null : voices.every(local) ? "local" : voices.every((id) => id.startsWith("elevenlabs:")) ? "elevenlabs" : "mixed";
  for (const id of ["free", "best", "private"] as ProfileId[]) {
    const p = PROFILE[id];
    if (p.brain === brain && (hears === null || hears === p.hear) && (speaks === null || speaks === p.speak || (id === "best" && speaks === "local"))) return id;
  }
  return null;
}

/** The keys a setup still needs. */
export function missingKeys(id: ProfileId, voice: VoiceInfo | null): string[] {
  return PROFILE[id].keys.filter((k) => !keyOf(k, voice));
}

export function keyOf(provider: string, voice: VoiceInfo | null): boolean {
  const i = store.info;
  if (provider === "anthropic") return !!i?.key;
  if (provider === "openai") return !!i?.openai_key;
  if (provider === "google") return !!i?.google_key;
  if (provider === "groq") return !!i?.groq_key;
  return !!voice?.providers.find((p) => p.id === provider)?.key;
}

/**
 * Put the settings in setup `id`: the brain, then the voice. The best
 * setup speaks with the ElevenLabs account's first voice when there is a
 * key, else with this Mac's voices until one is added.
 */
export async function applyProfile(id: ProfileId, voice: VoiceInfo): Promise<VoiceInfo> {
  const p = PROFILE[id];
  await store.saveSettings((s: AssistantSettings) => (s.provider = p.brain));
  // A plain copy: the page keeps the settings in reactive state, which the call can't clone.
  const now: VoiceSettings = JSON.parse(JSON.stringify(voice.settings));
  const next: VoiceSettings = { ...now, enabled: true, speak: true, live: "" };
  const whisper = voice.recommendation?.whisper ?? "small";
  if (id === "live") {
    next.live = now.live || LIVE_VOICE;
  } else {
    next.model = p.hear === "groq" ? GROQ_WHISPER : local(now.model) ? now.model : whisper;
    next.voices = { ...LOCAL_VOICES };
    if (p.speak === "elevenlabs" && keyOf("elevenlabs", voice)) {
      const kept = Object.values(now.voices).find((v) => v.startsWith("elevenlabs:"));
      const first = kept ?? (await call<AccountVoice[]>("elevenlabs_voices").then((l) => (l[0] ? `elevenlabs:voice/${l[0].voice_id}` : null), () => null));
      if (first) next.voices = { it: first, en: first };
    }
  }
  return call<VoiceInfo>("set_voice", { settings: next });
}
