// Claude Code sessions for the browser preview (`npm run dev`): one working
// through a plan step by step, one that keeps asking for a permission, one
// done. Shaped like the bridge's `agents` event.

import type { AgentSession, AgentStep, AgentsView, Approval } from "./monitor.svelte";

type Emit = (e: { kind: "agents" } & AgentsView) => void;

const STEP_MS = 2600;
const ASK_EVERY_MS = 45_000;
const WAIT_MS = 120_000;

const SCRIPT: Omit<AgentStep, "at">[] = [
  { kind: "read", tool: "Read", text: "menu.c", ok: true },
  { kind: "search", tool: "Grep", text: "swipe_close", ok: true },
  { kind: "edit", tool: "Edit", text: "menu.c", added: 14, removed: 3, ok: true },
  { kind: "edit", tool: "Edit", text: "gestures.c", added: 6, removed: 6, ok: true },
  { kind: "run", tool: "Bash", text: "idf.py build", ok: false },
  { kind: "edit", tool: "Edit", text: "gestures.h", added: 2, removed: 0, ok: true },
  { kind: "run", tool: "Bash", text: "idf.py build", ok: true },
  { kind: "write", tool: "Write", text: "FIRMWARE_CHANGELOG.md", added: 9, ok: true },
  { kind: "agent", tool: "Task", text: "Check the menu on the board", ok: true },
];

class PreviewAgents {
  #emit: Emit = () => {};
  #step = 0;
  #approval: Approval | null = null;
  #sessions: AgentSession[] = [];

  start(emit: Emit) {
    this.#emit = emit;
    const now = Date.now();
    this.#sessions = [
      { id: "yoru", project: "yoru", state: "work", task: "Close the settings menu with a swipe down", steps: [], since: now, updated: now },
      { id: "hub", project: "companion-hub", state: "work", task: "Add the agent feed to the hooks", steps: [{ kind: "read", tool: "Read", text: "hooks.rs", ok: true, at: now - 9000 }, { kind: "edit", tool: "Edit", text: "agents.rs", added: 212, removed: 0, ok: true, at: now - 4000 }], since: now - 60_000, updated: now - 4000 },
      { id: "dualeye", project: "esp32s3-dualeye-pcmonitor", state: "done", task: "Bump the firmware to 1.4.4", steps: [{ kind: "run", tool: "Bash", text: "idf.py build merge-bin", ok: true, at: now - 300_000 }, { kind: "write", tool: "Write", text: "version.txt", added: 1, ok: true, at: now - 280_000 }], since: now - 270_000, updated: now - 270_000 },
    ];
    setTimeout(() => this.#ask(), 4000);
    setInterval(() => this.#ask(), ASK_EVERY_MS);
    setInterval(() => this.#next(), STEP_MS);
    this.#send();
  }

  /** The working session takes its next step; at the end of the plan it's done for a while. */
  #next() {
    const s = this.#sessions[0];
    const now = Date.now();
    if (this.#step >= SCRIPT.length + 4) {
      this.#step = 0;
      Object.assign(s, { state: "work", steps: [], since: now });
    } else if (this.#step === SCRIPT.length) {
      Object.assign(s, { state: "done", since: now });
    }
    if (this.#step < SCRIPT.length) {
      s.steps = [...s.steps, { ...SCRIPT[this.#step], at: now }].slice(-8);
      s.updated = now;
    }
    this.#step++;
    this.#send();
  }

  #ask() {
    if (this.#approval) return;
    const now = Date.now();
    this.#approval = { id: `p${now}`, session: "hub", project: "companion-hub", tool: "Bash", kind: "run", text: "cargo test --workspace", detail: "Run the core tests", asked: now, expires: now + WAIT_MS };
    const hub = this.#sessions[1];
    Object.assign(hub, { state: "wait", waiting: "Bash: cargo test --workspace", since: now, updated: now });
    setTimeout(() => this.#approval?.asked === now && this.#settle(null), WAIT_MS);
    this.#send();
  }

  answer(id: string, allow: boolean): boolean {
    if (this.#approval?.id !== id) return false;
    this.#settle(allow);
    return true;
  }

  #settle(allow: boolean | null) {
    const now = Date.now();
    const hub = this.#sessions[1];
    this.#approval = null;
    Object.assign(hub, { state: "work", waiting: undefined, since: now, updated: now });
    if (allow !== null) hub.steps = [...hub.steps, { kind: "run" as const, tool: "Bash", text: "cargo test --workspace", ok: allow, at: now }].slice(-8);
    this.#send();
  }

  #send() {
    const sessions = [...this.#sessions].sort((a, b) => b.updated - a.updated).map((s) => ({ ...s, steps: [...s.steps] }));
    this.#emit({ kind: "agents", sessions, approvals: this.#approval ? [{ ...this.#approval }] : [] });
  }
}

export const previewAgents = new PreviewAgents();
