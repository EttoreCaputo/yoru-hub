import type { Transcriber } from "./types";

/** The meeting transcription engine's prerequisites, independent of the
 * voice transcription used by the board: Parakeet downloaded, or the
 * service's key there. `transcribers` is null until known. A key's
 * presence isn't a guarantee that the provider will accept a request. */
export function transcriptionRequirement(engine: string, localModel: boolean | null, transcribers: Transcriber[] | null): "loading" | "local" | "key" | "unsupported" | null {
  if (engine === "local") return localModel === null ? "loading" : localModel ? null : "local";
  if (!transcribers) return "loading";
  const by = transcribers.find((t) => t.id === engine);
  return !by ? "unsupported" : by.ready ? null : "key";
}

/** The service's name in a `provider:model` id, for "add its key". */
export function serviceOf(engine: string): string {
  const names: Record<string, string> = { google: "Google", groq: "Groq", openai: "OpenAI" };
  return names[engine.split(":")[0]] ?? engine.split(":")[0];
}
