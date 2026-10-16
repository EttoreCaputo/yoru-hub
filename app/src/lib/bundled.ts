// The firmware bundled with the app, per kind of board: its version and its
// changelog, read from the repository's firmware/<kind>/ at build time (the
// images themselves are embedded by the Rust side).

import type { BoardKind } from "./monitor.svelte";

// Vite reads these options at build time: they must stay literals.
const versions = import.meta.glob<string>("../../../firmware/*/version.txt", { query: "?raw", import: "default", eager: true });
const changelogs = import.meta.glob<string>("../../../firmware/*/FIRMWARE_CHANGELOG.md", { query: "?raw", import: "default", eager: true });

/** `firmware/<kind>/<file>` → `{kind: contents}`. */
function byKind(files: Record<string, string>): Partial<Record<BoardKind, string>> {
  const out: Partial<Record<BoardKind, string>> = {};
  for (const [path, text] of Object.entries(files)) {
    const kind = /firmware\/([^/]+)\//.exec(path)?.[1];
    if (kind === "dualeye" || kind === "yoru") out[kind] = text;
  }
  return out;
}

/** The version of the firmware the app flashes on each kind of board. */
export const BUNDLED_VERSION: Partial<Record<BoardKind, string>> = Object.fromEntries(
  Object.entries(byKind(versions)).map(([kind, v]) => [kind, v.trim()]),
);

/** Each kind's FIRMWARE_CHANGELOG.md. */
export const BUNDLED_CHANGELOG = byKind(changelogs);
