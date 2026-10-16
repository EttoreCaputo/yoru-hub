// What Yoru is up to, for its home: the line in the island, the badge on the
// sphere, and the reaction the mirror plays. Built from what the app already
// gets (the snapshot, the voice state, the transcripts); nothing here is sent
// to the board except the reactions asked for.

import { monitor, type ClaudeState } from "../monitor.svelte";
import type { Badge } from "../YoruBoard.svelte";

/** How long "Claude Code is done" stays after a turn ends. */
const DONE_MS = 60_000;
/** Pokes closer than this count as one bout: the third one annoys it. */
const POKE_GAP_MS = 1200;

export type Tone = "listen" | "work" | "done" | "wait" | "music" | "calm" | "off";

class Activity {
  /** When Claude Code last went from working to not, while the app watched. */
  claudeDoneAt = $state(0);
  /** The reaction the mirror plays, and when it was asked for. */
  reaction = $state<{ name: string; at: number } | null>(null);
  #claude: ClaudeState | null = null;
  #pokes: number[] = [];

  /** Called with every snapshot's Claude state. */
  observeClaude(state: ClaudeState | undefined) {
    if (this.#claude === "work" && state && state !== "work") this.claudeDoneAt = Date.now();
    if (state === "work") this.claudeDoneAt = 0;
    this.#claude = state ?? null;
  }

  get claudeDone(): boolean {
    return this.claudeDoneAt > 0 && monitor.now - this.claudeDoneAt < DONE_MS;
  }

  get badge(): Badge | null {
    if (monitor.voice === "error") return "error";
    if (monitor.last?.timer?.state === "ring") return "wait";
    if (monitor.last?.claude?.state === "work") return "work";
    return this.claudeDone ? "done" : null;
  }

  /** Play `name` here and on the board. */
  react(name: string) {
    this.reaction = { name, at: Date.now() };
    void monitor.yoruReact(name).catch(() => {});
  }

  /** A click on the sphere: a poke, and three in a row annoy it, as a tap on the board does. */
  poke() {
    const now = Date.now();
    this.#pokes = [...this.#pokes.filter((t) => now - t < POKE_GAP_MS * 2), now];
    this.react(this.#pokes.length >= 3 ? "annoyed" : this.#pokes.length === 2 ? "surprised" : "happy");
    if (this.#pokes.length >= 3) this.#pokes = [];
  }
}

export const activity = new Activity();
