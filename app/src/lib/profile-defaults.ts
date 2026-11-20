/** Defaults represent capabilities implemented by this app, not all APIs
 * a provider might offer. Keep meeting transcription separate from voice. */
export type ProfileId = "google" | "openai" | "anthropic" | "groq";
export const PROFILES: ProfileId[] = ["google", "openai", "anthropic", "groq"];

export function profileDefaults(id: ProfileId, whisper: string) {
  const brainModel = { google: "gemini-4-hydrogen", openai: "gpt-6-luna", anthropic: "claude-opus-5-5", groq: "openai/gpt-oss-120b" }[id];
  const hearing = id === "openai" ? "openai:whisper-1" : id === "groq" ? "groq:whisper-large-v3-turbo" : whisper;
  return { brainModel, hearing, live: id === "google" ? "Puck" : "", meetingTranscriber: id === "google" ? "google:gemini-3.8-flash" : "local" };
}
