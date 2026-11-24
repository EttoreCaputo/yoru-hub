// What the window knows, kept current by the Rust side's events: the board,
// the assistant (inbox, conversation, settings, the cards waiting for an
// answer), the connectors, the watchers and the tasks.
//
// Pages read it and call its methods; the Rust side stays the source of
// truth, so every change goes there and comes back as an event or a reply.

import { call, inPreview, on, why } from "./api";
import { BUNDLED_VERSION } from "./bundled";
import { tabs } from "./tabs.svelte";
import { changelog, updateFor, type Update } from "./updates";
import type {
  ActiveAgent,
  AssistantInfo,
  AssistantSettings,
  BoardFirmware,
  BoardState,
  BridgeEvent,
  ChatLine,
  ChatTitle,
  ChatSummary,
  Conversations,
  Confirmation,
  DeviceInfo,
  ConnectorStatus,
  FlashEvent,
  Gaze,
  Link,
  Task,
  TaskEntry,
  Todo,
  Meeting,
  MeetingEvent,
  MeetingSources,
  Notification,
  Onboarding,
  PocketEvent,
  PocketStatus,
  PermissionKind,
  PermissionState,
  Permissions,
  Preset,
  ScheduleEvent,
  Status,
  Summary,
  TimersInfo,
  VoiceState,
  WatchEntry,
  Watcher,
} from "./types";

/** What Yoru on the Mac does (`yoru_core::notch::Event`). */
type MacEvent =
  | { event: "state"; state: string }
  | { event: "level"; level: number }
  | { event: "react"; name: string }
  | { event: "fetch"; state: string; source: string; kind: string }
  | { event: "error"; message: string };

export type Page = "home" | "inbox" | "connectors" | "tasks" | "routines" | "memory" | "meetings" | "todos" | "spoken" | "animations" | "settings";

/** Opening the port resets the S3: it's up again after about this long. */
const BOOT_MS = 1100;
const LOG_LINES = 300;
const SCHEDULE_LOG = 30;
/** How often the coding agents are looked at. */
const AGENTS_MS = 5000;
/** How long the words being said stay without a new draft or the chat line. */
const HEARING_MS = 15_000;

class Store {
  readonly preview = inPreview;
  page = $state<Page>("home");
  now = $state(Date.now());

  // Every board, and the one the window shows (the rest of "the board" below).
  boards = $state<DeviceInfo[]>([]);
  shown: DeviceInfo | null = $derived(this.boards.find((b) => b.current) ?? null);

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
  chatTitle = $state<ChatTitle | null>(null);
  /** Every conversation with a line, newest first. */
  conversations = $state<ChatSummary[]>([]);
  /** The one Yoru is in: what's said to the board or the notch goes there. */
  currentChat = $state("");
  /** What the user is saying, written so far, while they speak; gone once the line is in the chat. */
  hearing = $state("");
  #hearingTimer: ReturnType<typeof setTimeout> | undefined;
  /** A question typed and not answered yet. */
  asking = $state<string | null>(null);
  askError = $state("");
  /** The cards the assistant waits on, oldest first. */
  confirmations = $state<Confirmation[]>([]);
  timers = $state<TimersInfo | null>(null);
  connectors = $state<ConnectorStatus[]>([]);
  connectorsError = $state("");
  presets = $state<Preset[]>([]);
  watchers = $state<WatchEntry[]>([]);
  tasks = $state<TaskEntry[]>([]);
  schedule = $state<(ScheduleEvent & { at: number })[]>([]);
  onboarding = $state<Onboarding | null>(null);
  /** The Mac's permissions; `null` until read. */
  permissions = $state<Permissions | null>(null);
  /** The microphone is required: the window waits for it (PermissionGate). */
  micMissing = $derived(!!this.permissions && this.permissions.microphone !== "granted");

  // Meetings.
  /** The recording that runs. */
  recording = $state<Meeting | null>(null);
  /** Its tracks' loudness, 0 to 1. */
  levels = $state<Record<string, number>>({});
  recordingError = $state<{ message: string; code?: string } | null>(null);
  /** The last recording that finished, or meeting that got its transcript, for the Meetings page to pick up. */
  recorded = $state<Meeting | null>(null);
  /** What the window bar's Registra records, picked on the Meetings page:
   *  everything played (`system`), one app's bundle id, or `none`. */
  recordPlayed = $state("system");
  /** Its microphones; null until picked, then the Mac's default ones. */
  recordMics = $state<string[] | null>(null);
  recordTitle = $state("");
  recordBusy = $state<"starting" | "stopping" | null>(null);
  /** The Pocket recorder, or null where it can't run. */
  pocket = $state<PocketStatus | null>(null);
  /** The Pocket's dialog is open (from the sidebar, or its Settings page). */
  pocketShown = $state(false);
  /** Every meeting, newest first, for the Home's list and the tabs. */
  meetings = $state<Meeting[]>([]);
  /** How far each meeting's transcript is, 0 to 1, while it is made. */
  transcribing = $state<Record<string, number>>({});
  /** Open Settings > Intelligence at a row when arriving from another page. */
  settingsSection = $state<"summarize" | "transcribe" | null>(null);
  /** Why a meeting's transcript failed. */
  transcribeErrors = $state<Record<string, string>>({});
  /** The transcription model's download, 0 to 1, while it runs. */
  modelDownload = $state<number | null>(null);
  /** How far each meeting's summary is, 0 to 1, while it is written. */
  summarizing = $state<Record<string, number>>({});
  summaryErrors = $state<Record<string, string>>({});
  /** Every summarized meeting's to-dos, for the To-do page and its count. */
  todos = $state<Todo[]>([]);
  openTodos = $derived(this.todos.filter((x) => !x.done).length);
  /** The last summary written, for the Meetings page to pick up. */
  summarized = $state<{ id: string; summary: Summary } | null>(null);

  /** The coding agents' conversations being worked on now, and Yoru's runs that just ended. */
  agents = $state<ActiveAgent[]>([]);
  working = $derived(this.agents.filter((a) => a.state === "working").length);

  unread = $derived(this.inbox.filter((n) => !n.read).length);

  // Yoru on the Mac: the notch's session, which the Home talks through too.
  /** His voice's state: idle, starting, listening, thinking, speaking, error. */
  macVoice = $state<VoiceState>("idle");
  /** How loud the microphone is, 0 to 1, while he listens. */
  macLevel = $state(0);
  /** The last reaction he was asked to play. */
  macReaction = $state<{ name: string; at: number } | null>(null);
  macError = $state("");
  /** The Home's microphone button is on. */
  talking = $state(false);

  #started = false;

  start() {
    if (this.#started) return;
    this.#started = true;
    setInterval(() => (this.now = Date.now()), 1000);
    // Nothing tells when an agent writes: they're looked at every few seconds.
    void this.refreshAgents();
    setInterval(() => void this.refreshAgents(), AGENTS_MS);
    on<BridgeEvent>("bridge", (e) => this.#apply(e));
    on<FlashEvent>("flash", (e) => this.#applyFlash(e));
    on<DeviceInfo[]>("boards", (list) => (this.boards = list));
    on<Notification[]>("inbox", (items) => (this.inbox = items));
    on<TaskEntry[]>("tasks", (items) => (this.tasks = items));
    on<ChatLine[]>("chat", (lines) => {
      this.chat = lines;
      this.hearing = "";
    });
    on<ChatTitle>("chat_title", (title) => (this.chatTitle = title));
    on<Conversations>("conversations", (c) => this.#applyConversations(c));
    on<string>("hearing", (text) => {
      this.hearing = text;
      // No line came (nothing to answer, an error): the draft doesn't stay.
      clearTimeout(this.#hearingTimer);
      this.#hearingTimer = setTimeout(() => (this.hearing = ""), HEARING_MS);
    });
    on<ScheduleEvent>("schedule", (e) => this.#onSchedule(e));
    on<ConnectorStatus[]>("connectors", (list) => (this.connectors = list));
    on<AssistantInfo>("assistant", (info) => (this.info = info));
    on<MeetingEvent>("meeting", (e) => this.#onMeeting(e));
    on<MacEvent>("notch", (e) => this.#onMac(e));
    on<PocketEvent>("pocket", (e) => (e.kind === "status" ? (this.pocket = e.status) : this.#recorded(e.meeting)));
    void this.#load();
  }

  async #load() {
    const quiet = <T>(p: Promise<T>) => p.catch(() => undefined);
    const [status, info, inbox, chat, confirmations, onboarding] = await Promise.all([
      quiet(call<Status>("status")),
      quiet(call<AssistantInfo>("assistant_info")),
      quiet(call<Notification[]>("inbox")),
      quiet(call<ChatLine[]>("chat")),
      quiet(call<Confirmation[]>("confirmations")),
      quiet(call<Onboarding>("onboarding")),
    ]);
    if (status) this.#applyStatus(status);
    if (info) this.info = info;
    if (inbox) this.inbox = inbox;
    if (chat) this.chat = chat;
    this.chatTitle = (await quiet(call<ChatTitle>("chat_title"))) ?? null;
    const conversations = await quiet(call<Conversations>("conversations"));
    if (conversations) this.#applyConversations(conversations);
    void this.refreshMeetings();
    if (confirmations) this.confirmations = confirmations;
    if (onboarding) this.onboarding = onboarding;
    void this.refreshPermissions();
    void this.refreshConnectors();
    void this.refreshWatchers();
    void this.refreshTimers();
    call<TaskEntry[]>("tasks").then((t) => (this.tasks = t), () => {});
    call<Preset[]>("connector_presets").then((p) => (this.presets = p), () => {});
    call<PocketStatus | null>("pocket_status").then((p) => (this.pocket = p), () => {});
    call<Meeting | null>("meeting_active").then((m) => (this.recording = m), () => {});
    call<string[]>("meetings_transcribing").then((ids) => {
      this.transcribing = Object.fromEntries(ids.filter((id) => !id.startsWith("summary:")).map((id) => [id, 0]));
      this.summarizing = Object.fromEntries(ids.filter((id) => id.startsWith("summary:")).map((id) => [id.slice(8), 0]));
    }, () => {});
  }

  #applyConversations(c: Conversations) {
    this.conversations = c.list;
    this.currentChat = c.current;
    // A conversation deleted (or a new one left empty) closes its tab.
    tabs.prune((t) => t.kind === "chat" && t.id !== c.current && !c.list.some((x) => x.id === t.id));
  }

  /** A meeting recorded, transcribed or summarized: the list follows. */
  #recorded(m: Meeting) {
    this.recorded = m;
    this.meetings = [m, ...this.meetings.filter((x) => x.id !== m.id)].sort((a, b) => b.id.localeCompare(a.id));
  }

  async refreshMeetings() {
    try {
      this.meetings = await call<Meeting[]>("meetings_list");
      tabs.prune((t) => t.kind === "meeting" && !this.meetings.some((m) => m.id === t.id));
    } catch {
      // The list stays as it was.
    }
    void this.refreshTodos();
  }

  async refreshTodos() {
    this.todos = (await call<Todo[]>("meeting_todos").catch(() => null)) ?? this.todos;
  }

  /** Tick a meeting's to-do off, or open it again. */
  async setTodo(todo: Todo, done: boolean) {
    this.todos = this.todos.map((x) => (x === todo ? { ...x, done } : x));
    this.todos = await call<Todo[]>("set_meeting_todo", { id: todo.meeting, text: todo.item.text, done });
  }

  // Conversations.

  /** A new conversation in a tab of its own (the one in use, if nothing was said in it yet). */
  async newChat() {
    const id = await call<string>("new_conversation");
    this.currentChat = id;
    tabs.open({ kind: "chat", id });
    return id;
  }

  /** `text` as the start of a new conversation, in a new tab. */
  async askNew(text: string) {
    if (!text.trim() || this.asking) return;
    const id = await this.newChat();
    await this.ask(text, false, id);
  }

  #onMeeting(e: MeetingEvent) {
    if (e.kind === "started") this.recording = e.meeting;
    else if (e.kind === "level") this.levels = e.levels;
    else if (e.kind === "error") this.recordingError = { message: e.message, code: e.code };
    else if (e.kind === "progress") {
      // Loading the model counts as the start; the tracks share the bar.
      this.transcribing = { ...this.transcribing, [e.id]: e.phase === "load" ? 0 : e.fraction };
      const { [e.id]: _, ...rest } = this.transcribeErrors;
      this.transcribeErrors = rest;
    } else if (e.kind === "transcribed" || e.kind === "failed") {
      const id = e.kind === "failed" ? e.id : e.meeting.id;
      const { [id]: _, ...rest } = this.transcribing;
      this.transcribing = rest;
      if (e.kind === "failed") this.transcribeErrors = { ...this.transcribeErrors, [id]: e.message };
      else this.#recorded(e.meeting);
    } else if (e.kind === "model") this.modelDownload = e.fraction;
    else if (e.kind === "todos") void this.refreshTodos();
    else if (e.kind === "summarizing") {
      this.summarizing = { ...this.summarizing, [e.id]: e.fraction };
      const { [e.id]: _, ...rest } = this.summaryErrors;
      this.summaryErrors = rest;
    } else if (e.kind === "summarized" || e.kind === "summary_failed") {
      const id = e.kind === "summary_failed" ? e.id : e.meeting.id;
      const { [id]: _, ...rest } = this.summarizing;
      this.summarizing = rest;
      if (e.kind === "summary_failed") this.summaryErrors = { ...this.summaryErrors, [id]: e.message };
      else {
        this.#recorded(e.meeting);
        this.summarized = { id, summary: e.summary };
        void this.refreshTodos();
      }
    }
    else {
      this.recording = null;
      this.recordBusy = null;
      this.levels = {};
      this.#recorded(e.meeting);
    }
  }

  #applyStatus(s: Status) {
    this.link = s.link;
    this.port = s.port;
    this.message = s.message ?? "";
    this.boardId = s.board;
    this.firmware = s.firmware;
    this.voice = s.voice ?? "idle";
    this.logs = s.logs;
    if (s.boards) this.boards = s.boards;
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
      case "confirmations":
        this.confirmations = e.cards as Confirmation[];
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

  /** Starts recording what was picked on the Meetings page. A failure is
   *  shown there, so that's where it goes. */
  async record() {
    if (this.recording || this.recordBusy) return;
    this.recordBusy = "starting";
    this.recordingError = null;
    try {
      if (!this.recordMics) {
        const sources = await call<MeetingSources>("meeting_sources");
        this.recordMics = sources.microphones.filter((m) => m.default).map((m) => m.id);
      }
      const played = this.recordPlayed;
      this.recording = await call<Meeting>("meeting_start", {
        request: { system: played === "system", app: played === "system" || played === "none" ? null : played, microphones: this.recordMics, title: this.recordTitle.trim() || null },
      });
      this.recordTitle = "";
    } catch (e) {
      this.recordingError = { message: why(e) };
      this.go("meetings");
    } finally {
      this.recordBusy = null;
    }
  }

  /** Ends it; the busy state lasts until the meeting's `stopped` event. */
  async stopRecording() {
    if (!this.recording || this.recordBusy) return;
    this.recordBusy = "stopping";
    try {
      await call("meeting_stop");
    } catch (e) {
      this.recordBusy = null;
      this.recordingError = { message: why(e) };
    }
  }

  go(page: Page) {
    this.page = page;
    tabs.page();
  }

  #onMac(e: MacEvent) {
    if (e.event === "state") {
      this.macVoice = (e.state as VoiceState) ?? "idle";
      if (this.macVoice !== "error") this.macError = "";
      if (this.macVoice !== "listening") this.macLevel = 0;
      // The turn ended on its own (an error, Yoru stopped): the button is off.
      if (this.talking && (this.macVoice === "idle" || this.macVoice === "error")) this.talking = false;
    } else if (e.event === "level") this.macLevel = e.level;
    else if (e.event === "react") this.macReaction = { name: e.name, at: Date.now() };
    else if (e.event === "error") {
      this.macError = e.message;
      this.macVoice = "error";
      this.talking = false;
    }
  }

  /** Talk to Yoru through the Mac's microphone: on, then off to send what was said. */
  talk(on: boolean) {
    if (this.talking === on) return;
    this.talking = on;
    this.macError = "";
    void call("notch_talk", { pressed: on }).catch((e) => {
      this.talking = false;
      this.macError = why(e);
    });
  }

  /** Stop what Yoru on the Mac says or hears. */
  hush() {
    this.talking = false;
    void call("notch_hush").catch(() => {});
  }

  // The assistant.

  /** `notch`: typed in the notch, whose tools it then uses instead of the board's. */
  /** `conversation`: the one it goes on (Yoru's current one when left out). */
  async ask(text: string, notch = false, conversation?: string) {
    text = text.trim();
    if (!text || this.asking) return;
    this.asking = text;
    this.askError = "";
    if (conversation) this.currentChat = conversation;
    try {
      await call("ask", { text, notch, conversation: conversation ?? null });
    } catch (e) {
      this.askError = why(e);
    } finally {
      this.asking = null;
    }
  }

  async confirm(id: string, allow: boolean) {
    await call<boolean>("confirm", { id, allow });
    this.confirmations = this.confirmations.filter((c) => c.id !== id);
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

  async putTask(task: Task) {
    this.tasks = await call<TaskEntry[]>("put_task", { task });
  }

  async removeTask(id: string) {
    this.tasks = await call<TaskEntry[]>("remove_task", { id });
  }

  async runTask(id: string) {
    await call<string>("run_task_now", { id });
  }

  async refreshAgents() {
    this.agents = (await call<ActiveAgent[]>("coding_active").catch(() => null)) ?? this.agents;
  }

  async stopAgent(run: number) {
    await call<string>("coding_stop", { run }).catch(() => {});
    await this.refreshAgents();
  }

  async refreshTimers() {
    this.timers = (await call<TimersInfo>("timers_info").catch(() => null)) ?? this.timers;
  }

  // The boards.

  /** What to call a board: its name, else "Yoru" and the end of its id. */
  boardName(b: DeviceInfo | null): string {
    if (!b) return "Yoru";
    return b.name ?? (b.id ? `Yoru ${b.id.slice(-4)}` : "Yoru");
  }

  /** Show `b` in the window: its state, firmware, WiFi and settings. */
  async showBoard(b: DeviceInfo) {
    this.boards = await call<DeviceInfo[]>("show_board", { board: b.target });
    const s = await call<Status>("status").catch(() => null);
    if (s) {
      this.#applyStatus(s);
      this.voice = s.voice ?? "idle";
    }
  }

  async setPrimary(b: DeviceInfo | null) {
    this.boards = await call<DeviceInfo[]>("set_primary", { board: b?.id ?? null });
  }

  async renameBoard(b: DeviceInfo, name: string) {
    if (!b.id) return;
    this.boards = await call<DeviceInfo[]>("rename_board", { id: b.id, name });
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

  async refreshPermissions() {
    const p = await call<Permissions>("permissions").catch(() => null);
    if (p) this.permissions = p;
    return this.permissions;
  }

  /**
   * macOS's prompt, also for one turned down (the app forgets the old answer
   * first). System Settings only when that can't help: a profile holds it,
   * or it was turned down again.
   */
  async requestPermission(kind: PermissionKind) {
    const before = this.permissions?.[kind];
    const after =
      before === "restricted" ? before : await call<PermissionState>("request_permission", { kind }).catch(() => null);
    if (after === "restricted" || (before === "denied" && after === "denied")) {
      await call("open_privacy", { kind }).catch(() => {});
    }
    await this.refreshPermissions();
  }

  async finishOnboarding() {
    await call("set_onboarding", { done: true, tourVersion: null });
    this.onboarding = { done: true, tour_version: this.onboarding?.tour_version ?? 0 };
  }
}

export const store = new Store();
