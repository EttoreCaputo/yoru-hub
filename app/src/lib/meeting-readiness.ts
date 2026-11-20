/** The meeting transcription engine's prerequisites, independent of the
 * voice transcription used by the board. A key's presence isn't a guarantee
 * that the provider will accept a request. */
export function transcriptionRequirement(engine: string, localModel: boolean | null, googleKey: boolean): "loading" | "local" | "google" | "unsupported" | null {
  if (engine === "local") return localModel === null ? "loading" : localModel ? null : "local";
  if (engine.startsWith("google:") && engine.length > "google:".length) return googleKey ? null : "google";
  return "unsupported";
}
