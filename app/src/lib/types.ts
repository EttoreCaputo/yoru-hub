// What the app's Rust side sends and takes, as TypeScript. Field names are
// the serde ones (snake_case).

export type Urgency = "low" | "normal" | "high" | "urgent";

export type Notification = {
  id: string;
  source: string;
  title: string;
  text: string;
  urgency: Urgency;
  url?: string;
  /** Unix ms. */
  at: number;
  read: boolean;
  /** The watcher that gave it. */
  by?: string;
};

/** `asked`: what the call was about in a few words (a search's query). */
export type Action = { tool: string; asked?: string; ok: boolean; summary: string };
export type Channel = "voice" | "text";
/** A coding agent's conversation being worked on now, or a run of Yoru's that just ended. */
export type ActiveAgent = {
  agent: "claude" | "codex" | "opencode";
  title: string;
  project: string;
  session: string | null;
  updated: number;
  run: number | null;
  state: "working" | "done" | "needs_you" | "failed" | "stopped";
  /** Its last step (`ran cargo build`), for Yoru's runs. */
  now: string | null;
};
export type ChatLine = { from: "you" | "yoru"; text: string; channel: Channel; actions?: Action[]; at: number };
/** The conversation's title: named by Yoru as it goes on, unless the user typed one. */
export type ChatTitle = { text: string; by_user: boolean; lines: number };
/** A conversation as the list shows it; an empty `title` isn't named yet (`preview` instead). */
export type ChatSummary = { id: string; title: string; by_user: boolean; started: number; updated: number; lines: number; preview: string };
/** Every conversation with a line, newest first, and the one Yoru is in (`conversations`). */
export type Conversations = { current: string; list: ChatSummary[] };
/** One conversation, to read (`conversation`). */
export type SavedChat = { id: string; title: ChatTitle; lines: ChatLine[] };
export type Reply = { text: string; actions: Action[]; model: string; elapsed_ms: number };

export type Routines = {
  recap: boolean;
  recap_at: string;
  recap_weekdays_only: boolean;
  recap_task: string;
  quiet: boolean;
  quiet_from: string;
  quiet_to: string;
};

export type Effort = "low" | "medium" | "high" | "xhigh" | "max";

export type Brain = "anthropic" | "openai" | "google" | "groq" | "ollama";

export type AssistantSettings = {
  name: string;
  /** null: the one with a key, Claude first. */
  provider: Brain | null;
  model: string;
  groq_model: string;
  openai_model: string;
  google_model: string;
  /** A model Ollama has (`qwen3:8b`); empty: none picked yet. */
  ollama_model: string;
  effort: Effort;
  task_effort: Effort;
  web_search: boolean;
  /** Ollama: let a model that can think do it, as long as it likes. */
  think: boolean;
  routines: Routines;
};

export type KeySource = "env" | "saved";
/** A model a service offers now (`provider_models`): `id` is `provider:model`; `known` when Yoru describes it. */
export type ListedModel = { id: string; model: string; name: string; note: string; chat: boolean; stt: boolean; known: boolean };

export type AssistantInfo = {
  settings: AssistantSettings;
  ready: boolean;
  brain: Brain | null;
  key: KeySource | null;
  groq_key: KeySource | null;
  groq_models: { id: string; note: string }[];
  openai_key: KeySource | null;
  tavily_key: KeySource | null;
  openai_models: { id: string; note: string }[];
  google_key: KeySource | null;
  google_models: { id: string; note: string }[];
  /** Groq's free limit, while it is waited out. */
  limited: string | null;
  dir: string | null;
  quiet_until: number | null;
  quiet_now: boolean;
};

export type WatchKind = { kind: "tool"; tool: string; arguments: unknown } | { kind: "task"; task: string };

export type Watcher = WatchKind & {
  id: string;
  name: string;
  every_min: number;
  enabled: boolean;
  instructions: string;
  max_urgency: Urgency;
};

export type WatchState = { last_run: number; last_error: string | null; told: number };
export type WatchEntry = { watcher: Watcher; state: WatchState };

export type ScheduleEvent =
  | { kind: "watched"; id: string; told: number; error: string | null }
  | { kind: "recap"; error: string | null }
  | { kind: "task"; id: string; error: string | null };

/** When a task runs (`assistant/tasks.rs`): days are 1 (Monday) to 7, none for every day. */
export type TaskSchedule =
  | { repeat: "once"; date: string; at: string }
  | { repeat: "daily"; at: string; days: number[] }
  | { repeat: "monthly"; day: number; at: string }
  | { repeat: "hourly"; every: number };

/** How far a task may go: read and tell only, ask before acting, or act freely. */
export type Acting = "never" | "ask" | "free";

/** A job the user gave Yoru to do on his own. */
export type Task = TaskSchedule & { id: string; name: string; what: string; enabled: boolean; acting: Acting };

export type TaskState = { last_run: number; last_error: string | null; last_result: string; runs: number; missed: number };
export type TaskEntry = { task: Task; state: TaskState; next: number | null; running: boolean };

export type ConnectionState = "disabled" | "needs_login" | "connecting" | "connected" | "error";

export type ToolInfo = {
  name: string;
  qualified_name: string;
  title?: string;
  description?: string;
  input_schema: Record<string, unknown>;
  enabled: boolean;
  acts: boolean;
  destructive: boolean;
};

export type ConnectorStatus = {
  id: string;
  name: string;
  enabled: boolean;
  state: ConnectionState;
  /** With `state: "error"`. */
  message?: string;
  /** `google`: Gmail or Calendar, served by Yoru Hub itself. */
  transport: "http" | "stdio" | "google";
  target: string;
  signed_in: boolean;
  tools: ToolInfo[];
};

export type Registration = "dynamic" | "own_client" | "none";

export type Preset = {
  id: string;
  name: string;
  url: string;
  registration: Registration;
  /** The user's own client needs a secret too (Advanced). */
  needs_client_secret: boolean;
  /** The app ships its own OAuth client for this `own_client` service. */
  app_client: boolean;
  docs: string;
  notes: string;
  /** How to make one's own client, for `own_client`. */
  setup: string;
  scopes: string[];
  auth_params: [string, string][];
  redirect_port: number | null;
  /** A Google service Yoru Hub serves itself, with the user's own client. */
  google: "gmail" | "calendar" | null;
};

export type ConnectorConfig = {
  id: string;
  name: string;
  transport: { type: "http"; url: string } | { type: "google"; service: "gmail" | "calendar" } | { type: "stdio"; command: string; args?: string[]; env?: Record<string, string | { secret: string }> };
  oauth?: { client_id?: string; client_secret?: string; scopes?: string[] };
  enabled: boolean;
};

/** A card the assistant waits on before it acts. */
export type Confirmation = {
  id: string;
  tool: string;
  text: string;
  detail?: string;
  asked: number;
  expires: number;
};

export type TimerKind = "timer" | "work" | "break" | "reminder";
export type TimerInfo = { id: number; kind: TimerKind; label?: string; total_s: number; left_s: number; state: "run" | "pause" | "ring"; ends_at?: string };
export type TimersInfo = { timers: TimerInfo[]; pomodoro: unknown; can_speak: boolean };

export type BoardFirmware =
  | { state: "version"; kind: "yoru"; version: string; idf: string | null }
  | { state: "missing" };

/** `starting` only in the notch: the Mac's microphone is still opening. */
export type VoiceState = "idle" | "starting" | "listening" | "thinking" | "speaking" | "error";
export type Link = "searching" | "connected" | "offline";
export type BoardState = "off" | "boot" | "waiting" | "live" | "stale";
export type Gaze = { x: number; y: number; movedAt: number };

/** One board the app knows: on USB, on WiFi, or still saying who it is. */
export type DeviceInfo = {
  /** Its session: a USB port, or `wifi:<id>`. */
  target: string;
  id: string | null;
  /** The name it was given, if any. */
  name: string | null;
  kind: "yoru" | null;
  /** `searching` until its `hello`, then `connected`. */
  link: Link;
  port: string | null;
  firmware: BoardFirmware | null;
  /** It says timers and alerts. */
  primary: boolean;
  /** The one the window shows. */
  current: boolean;
};

export type Status = {
  board: string | null;
  boards?: DeviceInfo[];
  primary?: string | null;
  link: Link;
  port: string | null;
  message: string | null;
  connected_age_ms: number | null;
  firmware: BoardFirmware | null;
  logs: string[];
  voice: VoiceState | null;
  port_setting: string | null;
  follow_pointer: boolean;
  notch: boolean;
};

export type Transport = "auto" | "usb" | "wifi";
export type WifiStatus = { enabled: boolean; ssid: string | null; state: string; ip: string | null; rssi: number | null; paired?: boolean };
export type WifiInfo = { transport: Transport; paired: { id: string; ssid: string | null }[]; board: WifiStatus | null; error: string | null };
export type WifiNetwork = { ssid: string; rssi: number; secure: boolean };
export type PortInfo = { name: string; vid: number; pid: number; product: string | null; is_board: boolean };

export type ImageInfo = { version: string; project: string; idf: string; built: string };
export type FirmwareInfo = { bundled: { kind: "yoru"; size: number; image: ImageInfo | null }[]; esptool: { python: string; version: string } | null };
export type FlashEvent = { kind: "log"; line: string } | { kind: "progress"; percent: number } | { kind: "setup"; message: string; percent: number | null };

export type YoruSetting = { value: number; min: number; max: number };
export type YoruSettings = Record<string, YoruSetting | string>;
export type BoardVoice = { volume: number | null; wake_sound: boolean | null };

export type SttLanguage = "auto" | "it" | "en";
export type VoiceSettings = {
  enabled: boolean;
  model: string;
  language: SttLanguage;
  keep_recordings: boolean;
  speak: boolean;
  follow_up: boolean;
  intent_model: string;
  turn_model: string;
  keep_warm: boolean;
  voices: Record<string, string>;
  llm: boolean;
  llm_model: string;
  /** Gemini Live's voice, which then hears and speaks itself; "" for off. */
  live: string;
};
export type ModelInfo = {
  id: string;
  kind: "whisper" | "voice" | "llm" | "intent" | "turn";
  language: string | null;
  engine: string | null;
  bytes: number;
  note: string;
  license: string;
  installed: boolean;
  provider: string | null;
};
export type ProviderInfo = { id: string; name: string; note: string; keys_url: string; key_env: string; key: KeySource | null };
export type SidecarState = "off" | "starting" | "ready" | "error";
export type VoiceInfo = {
  settings: VoiceSettings;
  models: ModelInfo[];
  stt: SidecarState;
  stt_error: string | null;
  piper: string | null;
  piper_install: string | null;
  tts: SidecarState;
  tts_error: string | null;
  llm: SidecarState;
  llm_error: string | null;
  /** Where llama.cpp's llama-server is: the language model and EmbeddingGemma run on it. */
  llm_server?: string | null;
  download: [string, number] | null;
  providers: ProviderInfo[];
  /** Gemini Live's voices: the name, and how Google describes it. */
  /** Name, Google's description, "male" or "female". */
  live_voices: [string, string, string][];
  /** The local models that suit this computer. */
  recommendation?: { whisper: string; llm: string | null; why: string };
};
export type AccountVoice = { voice_id: string; name: string; category: string; description: string | null };
export type KeyCheck = { state: "valid" | "invalid" | "unknown"; error: string | null };

export type McpInfo = { command: string | null; args: string[]; registered: boolean; config_path: string | null; hub_error: string | null };
/** A sentence Yoru said with ElevenLabs (`spoken_clips`). */
/** One of a Yoru's animation clips (`yoru_clips`): looping ones play until stopped. */
export type BoardClip = { name: string; seconds: number; loop: boolean };
export type SpokenClip = { id: string; at: number; text: string; language: string; voice: string; duration_ms: number };
/** A recorded track of a meeting: `others` is what the computer played, `mic` the user. */
export type MeetingTrack = { name: string; source: string; kind: "others" | "mic"; file: string | null; duration_ms: number; rate: number; channels: number };
/** A recording and what came of it (`meetings_list`). */
export type Meeting = {
  id: string;
  title: string | null;
  started_at: number;
  duration_ms: number;
  state: "recording" | "done" | "interrupted";
  tracks: MeetingTrack[];
  /** When its transcript was made. */
  transcribed_at: number | null;
  /** When its summary was made. */
  summarized_at: number | null;
  /** Where it came from when not recorded here: `pocket`. */
  origin?: string | null;
};
/** A meeting's summary (`meeting_summary`); `by` names the model. */
export type Summary = {
  title: string;
  overview: string;
  /** The body, as the style lays it out; `points` only in older summaries. */
  sections: { title: string; items: string[] }[];
  points?: string[];
  decisions: string[];
  actions: ActionItem[];
  questions: string[];
  /** `auto`, `meeting`, `call`, `interview`, `lecture`, `note`; empty in older summaries. */
  style: string;
  by: string;
  made_at: number;
};
/** A to-do from a meeting: `due` as said, `date` as `YYYY-MM-DD` when it can be told; `draft` the text of a message or an email. */
export type ActionItem = { text: string; owner: string; due: string; date: string; kind: "task" | "reminder" | "message" | "email"; draft: string };
/** A meeting's to-do on the To-do page (`meeting_todos`): the meeting it came from, and whether it's ticked off. */
export type Todo = { meeting: string; title: string; started_at: number; item: ActionItem; done: boolean };
/** `meeting_summarizers`: the brain's name (Yoru writes the summaries with it), and every model of a provider with a key, for the transcriber. */
export type Summarizers = { brain: string | null; models: { id: string; provider: string; model: string; note: string }[] };
/** A stretch of speech: `kind` `mic` is the user's track; `speaker` who, within the track (0: first heard). */
export type TranscriptLine = { track: string; kind: "mic" | "others"; start_ms: number; end_ms: number; text: string; speaker: number | null };
/** Someone heard: `key` is `<track>/<speaker or ->`; `number` 0 is the user, then 1, 2… as they first speak. */
export type Speaker = { key: string; number: number; name: string | null };
export type Transcript = { engine: string; language: string | null; lines: TranscriptLine[]; speakers: Speaker[] };
/** `meeting_settings`: when to transcribe, the language (empty: Parakeet finds it), whether to tell the speakers apart, and whether to notice meetings on their own (`detect`). */
export type MeetingSettings = { transcribe: "after" | "live" | "manual"; /** `local` (Parakeet) or a Gemini model (`google:gemini-3.8-flash`). */ transcribe_by: string; language: string; speakers: boolean; /** The default style of a summary (`Summary.style`). */ summary_style: string; auto_summary: boolean; detect: boolean };
/** What can be recorded right now (`meeting_sources`). */
export type MeetingSources = { apps: { id: string; name: string; playing: boolean; icon?: string }[]; microphones: { id: string; name: string; default: boolean }[] };
/** What to record (`meeting_start`): everything played, or one app's sound, and microphones. */
export type MeetingRequest = { system: boolean; app: string | null; microphones: string[]; title: string | null };
/** A running recording's news (`meeting` event). */
export type MeetingEvent =
  | { kind: "started"; meeting: Meeting }
  | { kind: "level"; levels: Record<string, number> }
  | { kind: "error"; message: string; code?: "no_system_audio" | "no_microphone_sound" }
  | { kind: "done"; meeting: Meeting }
  | { kind: "progress"; id: string; track: string; phase: "load" | "transcribe"; fraction: number }
  | { kind: "transcribed"; meeting: Meeting }
  | { kind: "failed"; id: string; message: string }
  | { kind: "model"; fraction: number }
  | { kind: "summarizing"; id: string; fraction: number }
  | { kind: "summarized"; meeting: Meeting; summary: Summary }
  | { kind: "summary_failed"; id: string; message: string }
  | { kind: "todos" };
export type Onboarding = { done: boolean; tour_version: number };
/** Where a Mac permission stands (`unknown`: never asked). */
export type PermissionState = "granted" | "denied" | "restricted" | "unknown" | "unsupported";
export type PermissionKind = "microphone" | "calendar" | "system_audio";
/** The microphone is required; the calendar and the system audio are for meetings. */
export type Permissions = Record<PermissionKind, PermissionState>;

/** A bridge event: `kind` and its fields, plus the board it comes from. */
export type BridgeEvent = { kind: string; target?: string | null; board?: string | null; [field: string]: unknown };

/** The Pocket recorder's settings (`pocket_settings`). */
export type PocketSettings = { enabled: boolean; auto_sync: boolean; delete_after_sync: boolean; set_clock: boolean };
/** A recording on the Pocket; `meeting` is the meeting it became. */
export type PocketRecording = { date: string; name: string; seconds: number; started_at: number | null; meeting: string | null };
/** The Pocket and its link (`pocket_status`, the `pocket` event). */
export type PocketStatus = {
  phase: "off" | "unpaired" | "pairing" | "searching" | "connecting" | "connected" | "refused" | "no_bluetooth";
  settings: PocketSettings;
  name: string | null;
  battery: number | null;
  firmware: string | null;
  wifi_firmware: string | null;
  free_mb: number | null;
  total_mb: number | null;
  recording: boolean;
  recordings: PocketRecording[];
  checking: boolean;
  transfer: { name: string; bytes: number; total: number; queued: number } | null;
  synced_at: number | null;
  error: string | null;
};
export type PocketEvent = { kind: "status"; status: PocketStatus } | { kind: "imported"; meeting: Meeting };
