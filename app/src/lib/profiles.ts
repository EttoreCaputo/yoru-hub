/** Guided provider setups. Advanced settings remain independently editable. */
import { call } from "./api";
import { profileDefaults, type ProfileId } from "./profile-defaults";
import { store } from "./store.svelte";
import type { MeetingSettings, VoiceInfo, VoiceSettings } from "./types";

export { PROFILES, type ProfileId } from "./profile-defaults";

export function keyOf(provider: ProfileId): boolean {
  const i = store.info;
  return !!(provider === "anthropic" ? i?.key : provider === "openai" ? i?.openai_key : provider === "google" ? i?.google_key : i?.groq_key);
}

/** Apply only after the user reviews the data destinations and limitations. */
export async function applyProfile(id: ProfileId, voice: VoiceInfo): Promise<VoiceInfo> {
  if (!keyOf(id)) throw new Error("The provider's API key is missing");
  const meetings = await call<MeetingSettings>("meeting_settings");
  if (!store.info) throw new Error("Assistant settings are not available");
  const defaults = profileDefaults(id, voice.recommendation?.whisper ?? "small");
  const next: VoiceSettings = JSON.parse(JSON.stringify(voice.settings));
  next.enabled = true;
  next.speak = true;
  next.live = defaults.live;
  next.model = defaults.hearing;
  next.voices = { it: "it_IT-paola-medium", en: "en_GB-alba-medium" };
  // A meeting can only use Google's audio API or local Parakeet here.
  const transcribe_by = defaults.meetingTranscriber;
  await store.saveSettings((s) => {
    s.provider = id;
    if (id === "google") s.google_model = defaults.brainModel;
    else if (id === "openai") s.openai_model = defaults.brainModel;
    else if (id === "anthropic") s.model = defaults.brainModel;
    else s.groq_model = defaults.brainModel;
  });
  const result = await call<VoiceInfo>("set_voice", { settings: next });
  await call("set_meeting_settings", { settings: { ...meetings, transcribe_by } });
  return result;
}

const LOCAL_VOICES = ["it_IT-paola-medium", "en_GB-alba-medium"];

/** What a profile runs on this Mac (meeting transcription, hearing for
 * Claude, Piper's voices), downloaded where missing. Google needs none. */
export async function prepareLocal(id: ProfileId, voice: VoiceInfo): Promise<void> {
  if (id === "google") return;
  if ((await call<boolean>("meeting_model").catch(() => true)) === false) await call("meeting_model_download");
  const whisper = voice.recommendation?.whisper ?? "small";
  if (id === "anthropic" && !voice.models.some((m) => m.id === whisper && m.installed)) await call("download_model", { id: whisper });
  if (!voice.piper) await call("install_engine", { engine: "piper" });
  for (const v of LOCAL_VOICES) {
    if (!voice.models.some((m) => m.id === v && m.installed)) await call("download_model", { id: v });
  }
}
