// What the window knows, kept current by the Rust side's events: the board,
// the assistant (inbox, conversation, settings), Claude Code's sessions and
// the cards waiting for an answer, the connectors and the watchers.
//
// Pages read it and call its methods; the Rust side stays the source of
// truth, so every change goes there and comes back as an event or a reply.

import { call, inPreview, on, why } from "./api";
import { BUNDLED_VERSION } from "./bundled";
import { changelog, updateFor, type Update } from "./updates";
import type {
  AgentsView,
  Approval,
  AssistantInfo,
  AssistantSettings,
  BoardFirmware,
  BoardState,
  BridgeEvent,
  ChatLine,
  ConnectorStatus,
  FlashEvent,
  Gaze,
  Link,
  Notification,
  Onboarding,
  Preset,
  ScheduleEvent,
  Status,
  TimersInfo,
  VoiceState,
  WatchEntry,
  Watcher,
} from "./types";

export type Page = "today" | "conversation" | "inbox" | "connectors" | "routines" | "memory" | "yoru" | "settings";

/** Opening the port resets the S3: it's up again after about this long. */
const BOOT_MS = 1100;
const LOG_LINES = 300;
const SCHEDULE_LOG = 30;

class Store {
  readonly preview = inPreview;
  page = $state<Page>("today");
  now = $state(Date.now());

  // The board.
  link = $state<Link>("searching");
  port = $state<string | null>(null);
  message = $state("");
  permissionDenied = $state(false);
  boardId = $state<string | null>(null);
  connectedAt = $state(0);
  firmware = $state<BoardFirmware | null>(null);
  voice = $state<VoiceState>("idle");
  logs = $state<string[]>([]);
  gaze = $state<Gaze | null>(null);
  boardState: BoardState = $derived(this.link !== "connected" ? "off" : this.now - this.connectedAt < BOOT_MS ? "boot" : "live");
  /** Set when the board runs older firmware than the app's. */
  update: Update | null = $derived(this.link === "connected" ? updateFor(this.firmware, BUNDLED_VERSION, changelog()) : null);

  // Flashing.
  job = $state<"idle" | "identify" | "flash">("idle");
  flashPercent = $state(0);
  flashLog = $state<string[]>([]);
  flashError = $state("");
  flashedAt = $state(0);
  setup = $state<{ message: string; percent: number | null } | null>(null);

  // The assistant.
  info = $state<AssistantInfo | null>(null);
  inbox = $state<Notification[]>([]);
  chat = $state<ChatLine[]>([]);
  /** A question typed and not answered yet. */
  asking = $state<string | null>(null);
  askError = $state("");
  agents = $state<AgentsView>({ sessions: [], approvals: [] });
  timers = $state<TimersInfo | null>(null);
  connectors = $state<ConnectorStatus[]>([]);
  connectorsError = $state("");
  presets = $state<Preset[]>([]);
  watchers = $state<WatchEntry[]>([]);
  schedule = $state<(ScheduleEvent & { at: number })[]>([]);
  onboarding = $state<Onboarding | null>(null);

  unread = $derived(this.inbox.filter((n) => !n.read).length);
  /** The cards Yoru itself is waiting on (its session is `yoru`), and Claude Code's. */
  yoruApprovals = $derived(this.agents.approvals.filter((a) => a.session === "yoru"));
  codeApprovals = $derived(this.agents.approvals.filter((a) => a.session !== "yoru"));

  #started = false;

  start() {
    if (this.#started) return;
    this.#started = true;
    setInterval(() => (this.now = Date.now()), 1000);
    on<BridgeEvent>("bridge", (e) => this.#apply(e));
    on<FlashEvent>("flash", (e) => this.#applyFlash(e));
    on<Notification[]>("inbox", (items) => (this.inbox = items));
    on<ChatLine[]>("chat", (lines) => (this.chat = lines));
    on<ScheduleEvent>("schedule", (e) => this.#onSchedule(e));
    on<ConnectorStatus[]>("connectors", (list) => (this.connectors = list));
    on<AssistantInfo>("assistant", (info) => (this.info = info));
    void this.#load();
  }

  async #load() {
    const quiet = <T>(p: Promise<T>) => p.catch(() => undefined);
    const [status, info, inbox, chat, agents, onboarding] = await Promise.all([
      quiet(call<Status>("status")),
      quiet(call<AssistantInfo>("assistant_info")),
      quiet(call<Notification[]>("inbox")),
      quiet(call<ChatLine[]>("chat")),
      quiet(call<AgentsView>("claude_agents")),
      quiet(call<Onboarding>("onboarding")),
    ]);
    if (status) this.#applyStatus(status);
    if (info) this.info = info;
    if (inbox) this.inbox = inbox;
    if (chat) this.chat = chat;
    if (agents) this.agents = agents;
    if (onboarding) this.onboarding = onboarding;
    void this.refreshConnectors();
    void this.refreshWatchers();
    void this.refreshTimers();
    call<Preset[]>("connector_presets").then((p) => (this.presets = p), () => {});
  }

  #applyStatus(s: Status) {
    this.link = s.link;
    this.port = s.port;
    this.message = s.message ?? "";
    this.boardId = s.board;
    this.firmware = s.firmware;
    this.voice = s.voice ?? "idle";
    this.logs = s.logs;
    if (s.link === "connected") this.connectedAt = Date.now() - (s.connected_age_ms ?? BOOT_MS);
  }

  #apply(e: BridgeEvent) {
    switch (e.kind) {
      case "waiting":
        this.link = "searching";
        this.message = String(e.reason ?? "");
        break;
      case "connected":
        this.link = "connected";
        this.port = String(e.port ?? "");
        this.boardId = e.board ?? this.boardId;
        this.message = "";
        this.permissionDenied = false;
        this.connectedAt = Date.now();
        this.firmware = null;
        this.voice = "idle";
        break;
      case "firmware":
        this.firmware = e.firmware as BoardFirmware;
        break;
      case "board_log":
        this.logs.push(String(e.line));
        if (this.logs.length > LOG_LINES) this.logs.splice(0, this.logs.length - LOG_LINES);
        break;
      case "voice_state":
        this.voice = e.state as VoiceState;
        break;
      case "agents":
        this.agents = { sessions: e.sessions as AgentsView["sessions"], approvals: e.approvals as Approval[] };
        break;
      case "timer_fired":
        void this.refreshTimers();
        break;
      case "disconnected":
        this.link = "offline";
        this.voice = "idle";
        this.message = String(e.reason ?? "");
        this.permissionDenied = !!e.permission_denied;
        break;
    }
  }

  #applyFlash(e: FlashEvent) {
    if (e.kind === "progress") {
      this.setup = null;
      this.flashPercent = e.percent;
    } else if (e.kind === "setup") {
      this.setup = { message: e.message, percent: e.percent };
      if (e.percent === null) this.flashLog.push(e.message);
    } else {
      this.flashLog.push(e.line);
    }
  }

  #onSchedule(e: ScheduleEvent) {
    this.schedule = [{ ...e, at: Date.now() }, ...this.schedule].slice(0, SCHEDULE_LOG);
    if (e.kind === "watched") void this.refreshWatchers();
  }

  go(page: Page) {
    this.page = page;
  }

  // The assistant.

  async ask(text: string) {
    text = text.trim();
    if (!text || this.asking) return;
    this.asking = text;
    this.askError = "";
    try {
      await call("ask", { text });
    } catch (e) {
      this.askError = why(e);
    } finally {
      this.asking = null;
    }
  }

  async approve(id: string, allow: boolean) {
    await call<boolean>("claude_approve", { id, allow });
    this.agents = { ...this.agents, approvals: this.agents.approvals.filter((a) => a.id !== id) };
  }

  async saveSettings(change: (s: AssistantSettings) => void) {
    if (!this.info) return;
    const settings = structuredClone($state.snapshot(this.info.settings)) as AssistantSettings;
    change(settings);
    this.info = await call<AssistantInfo>("set_assistant_settings", { settings });
  }

  async doNotDisturb(minutes: number) {
    this.info = await call<AssistantInfo>("set_do_not_disturb", { minutes });
  }

  async refreshInfo() {
    this.info = await call<AssistantInfo>("assistant_info");
  }

  // The inbox.

  read = (id: string) => call("inbox_read", { id });
  readAll = () => call("inbox_read_all");
  dismiss = (id: string) => call("inbox_dismiss", { id });
  clearInbox = () => call("inbox_clear");
  open = (id: string) => call("open_notification", { id });

  // Connectors and watchers.

  async refreshConnectors() {
    try {
      this.connectors = await call<ConnectorStatus[]>("connectors");
      this.connectorsError = "";
    } catch (e) {
      this.connectorsError = why(e);
    }
  }

  async refreshWatchers() {
    this.watchers = (await call<WatchEntry[]>("watchers").catch(() => null)) ?? this.watchers;
  }

  async putWatcher(watcher: Watcher) {
    this.watchers = await call<WatchEntry[]>("put_watcher", { watcher });
  }

  async removeWatcher(id: string) {
    this.watchers = await call<WatchEntry[]>("remove_watcher", { id });
  }

  async refreshTimers() {
    this.timers = (await call<TimersInfo>("timers_info").catch(() => null)) ?? this.timers;
  }

  // The board.

  async flash() {
    if (this.job !== "idle") return;
    this.job = "flash";
    this.flashPercent = 0;
    this.flashLog = [];
    this.flashError = "";
    this.flashedAt = 0;
    try {
      await call("flash_board", { port: null, kind: "yoru" });
      this.flashedAt = Date.now();
    } catch (e) {
      this.flashError = why(e);
    } finally {
      this.job = "idle";
      this.setup = null;
    }
  }

  async finishOnboarding() {
    await call("set_onboarding", { done: true, tourVersion: null });
    this.onboarding = { done: true, tour_version: this.onboarding?.tour_version ?? 0 };
  }
}

export const store = new Store();
