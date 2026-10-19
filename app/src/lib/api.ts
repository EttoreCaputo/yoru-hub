// The one door to the Rust side: its commands and its events. Outside Tauri
// (`npm run dev` in a browser) a mocked Yoru answers instead (`preview.ts`),
// so the pages can be looked at without the app.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { preview } from "./preview";

export const inPreview = !isTauri();

export function call<T = void>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return inPreview ? preview.call<T>(cmd, args) : invoke<T>(cmd, args);
}

/** Listen to `event` for the life of the app. */
export function on<T>(event: string, handler: (payload: T) => void) {
  if (inPreview) preview.on(event, handler as (p: unknown) => void);
  else void listen<T>(event, (e) => handler(e.payload));
}

/** An error from a command, as words. */
export const why = (e: unknown) => (e instanceof Error ? e.message : String(e));
