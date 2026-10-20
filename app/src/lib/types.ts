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

export type Action = { tool: string; ok: boolean; summary: string };
export type Channel = "voice" | "text";
export type ChatLine = { from: "you" | "yoru"; text: string; channel: Channel; actions?: Action[]; at: number };
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

export type Brain = "anthropic" | "openai" | "groq" | "ollama";

export type AssistantSettings = {
  name: string;
  /** null: the one with a key, Claude first. */
  provider: Brain | null;
  model: string;
  groq_model: string;
  openai_model: string;
  /** A model Ollama has (`qwen3:8b`); empty: none picked yet. */
  ollama_model: string;
  effort: Effort;
  task_effort: Effort;
  web_search: boolean;
  routines: Routines;
};

export type KeySource = "env" | "saved";

export type AssistantInfo = {
  settings: AssistantSettings;
  ready: boolean;
  brain: Brain | null;
  key: KeySource | null;
  groq_key: KeySource | null;
  groq_models: { id: string; note: string }[];
  openai_key: KeySource | null;
  openai_models: { id: string; note: string }[];
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

export type ScheduleEvent = { kind: "watched"; id: string; told: number; error: string | null } | { kind: "recap"; error: string | null };

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
  transport: "http" | "stdio";
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
};

export type ConnectorConfig = {
  id: string;
  name: string;
  transport: { type: "http"; url: string } | { type: "stdio"; command: string; args?: string[]; env?: Record<string, string | { secret: string }> };
  oauth?: { client_id?: string; client_secret?: string; scopes?: string[] };
  enabled: boolean;
};

export type AgentState = "work" | "wait" | "done" | "idle";
export type AgentStep = { kind: string; tool: string; text: string; at: number };
export type AgentSession = {
  id: string;
  project?: string;
  state: AgentState;
  task?: string;
  waiting?: string;
  steps: AgentStep[];
  since: number;
  updated: number;
};
export type Approval = {
  id: string;
  session: string;
  project?: string;
  tool: string;
  kind: string;
  text: string;
  detail?: string;
  asked: number;
  expires: number;
};
export type AgentsView = { sessions: AgentSession[]; approvals: Approval[] };

export type TimerKind = "timer" | "work" | "break" | "reminder";
export type TimerInfo = { id: number; kind: TimerKind; label?: string; total_s: number; left_s: number; state: "run" | "pause" | "ring"; ends_at?: string };
export type TimersInfo = { timers: TimerInfo[]; pomodoro: unknown; show_on: string; can_speak: boolean };

export type BoardFirmware =
  | { state: "version"; kind: "yoru"; version: string; idf: string | null; protocol: number }
  | { state: "legacy" }
  | { state: "missing" };

export type VoiceState = "idle" | "listening" | "thinking" | "speaking" | "error";
export type Link = "searching" | "connected" | "offline";
export type BoardState = "off" | "boot" | "waiting" | "live" | "stale";
export type Gaze = { x: number; y: number; movedAt: number };

export type Status = {
  board: string | null;
  link: Link;
  port: string | null;
  message: string | null;
  connected_age_ms: number | null;
  firmware: BoardFirmware | null;
  logs: string[];
  voice: VoiceState | null;
  port_setting: string | null;
  follow_pointer: boolean;
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
  keep_warm: boolean;
  voices: Record<string, string>;
  llm: boolean;
  llm_model: string;
  personality: string;
  personality_custom: string;
};
export type ModelInfo = {
  id: string;
  kind: "whisper" | "voice" | "llm";
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
  download: [string, number] | null;
  providers: ProviderInfo[];
};
export type AccountVoice = { voice_id: string; name: string; category: string; description: string | null };
export type KeyCheck = { state: "valid" | "invalid" | "unknown"; error: string | null };

export type ClaudeLink = { connected: boolean; chained: string | null; last_update_s: number | null; settings_path: string | null };
export type HooksStatus = { connected: boolean; last_event_s: number | null; settings_path: string | null };
export type AlertSettings = {
  needs_you: boolean;
  done: boolean;
  done_after_s: number;
  usage: boolean;
  speak: boolean;
  language: string;
  approvals: boolean;
  approval_wait_s: number;
};
export type ClaudeAlertsInfo = { settings: AlertSettings; hooks: HooksStatus; can_speak: boolean; last: { text: string; error: string | null; age_s: number } | null };
export type McpInfo = { command: string | null; args: string[]; registered: boolean; config_path: string | null; hub_error: string | null };
export type Onboarding = { done: boolean; tour_version: number };

/** A bridge event: `kind` and its fields, plus the board it comes from. */
export type BridgeEvent = { kind: string; target?: string | null; board?: string | null; [field: string]: unknown };
