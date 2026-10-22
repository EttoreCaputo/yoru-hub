// A mocked Yoru for the browser preview (`npm run dev`, no Tauri): enough
// state behind every command for each page to look and act like the app.
//
//   ?setup      opens the first-run setup
//   ?board=off  no board on the desk
//   ?key=none   no brain's key yet (no Groq, no Anthropic)
//   ?brain=groq Yoru thinks with Groq (its key only)

import type {
  AccountVoice,
  AssistantInfo,
  ChatLine,
  Confirmation,
  ConnectorConfig,
  ConnectorStatus,
  Notification,
  Preset,
  SpokenClip,
  TimersInfo,
  ToolInfo,
  VoiceInfo,
  WatchEntry,
  Watcher,
  YoruSettings,
} from "./types";

type Handler = (payload: unknown) => void;

const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
const MIN = 60_000;
const now = () => Date.now();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const tool = (name: string, description: string, acts = false, connector = "linear"): ToolInfo => ({
  name,
  qualified_name: `${connector}__${name}`,
  description,
  input_schema: { type: "object", properties: {} },
  enabled: true,
  acts,
  destructive: acts,
});

// Built as if the app shipped GitHub's client but not Slack's (Google always takes the user's own),
// so both the one-click path and Advanced show.
const preset = (id: string, name: string, url: string, notes: string, more: Partial<Preset> = {}): Preset => ({
  id, name, url, registration: "dynamic", needs_client_secret: false, app_client: false, docs: "https://modelcontextprotocol.io", notes, setup: "", scopes: [], auth_params: [], redirect_port: null, google: null, ...more,
});
const GOOGLE_SETUP = "In your own Google Cloud project: enable the API, set up the OAuth consent screen, then create an OAuth client of type \"Desktop app\".";
const google = { registration: "own_client", needs_client_secret: true, app_client: false, setup: GOOGLE_SETUP, docs: "https://developers.google.com/workspace/guides/create-credentials#desktop-app" } as const;
const presets: Preset[] = [
  preset("notion", "Notion", "https://mcp.notion.com/mcp", "Pages and databases of your workspace."),
  preset("linear", "Linear", "https://mcp.linear.app/mcp", "Issues, projects and comments."),
  preset("atlassian", "Jira and Confluence", "https://mcp.atlassian.com/v2/mcp", "Atlassian's Rovo MCP server for Jira and Confluence."),
  preset("sentry", "Sentry", "https://mcp.sentry.dev/mcp", "Issues and errors of your Sentry organizations."),
  preset("stripe", "Stripe", "https://mcp.stripe.com", "Payments, customers and invoices. Its tools can act on real money."),
  preset("todoist", "Todoist", "https://ai.todoist.net/mcp", "Tasks and projects."),
  preset("tavily", "Tavily", "https://mcp.tavily.com/mcp/", "Web search, with news and finance; gives Yoru the web on every brain. The free plan has 1,000 credits a month: a search costs 1 (2 when advanced)."),
  preset("exa", "Exa", "https://mcp.exa.ai/mcp", "Another web search, and pages read in full; no sign-in, with a free daily allowance.", { registration: "none" }),
  preset("airtable", "Airtable", "https://mcp.airtable.com/mcp", "Your bases, their tables and records."),
  preset("clickup", "ClickUp", "https://mcp.clickup.com/mcp", "Tasks, docs and chat of your workspace."),
  preset("miro", "Miro", "https://mcp.miro.com/", "Your boards: read them and add to them."),
  preset("cloudflare-docs", "Cloudflare docs", "https://docs.mcp.cloudflare.com/mcp", "Searches Cloudflare's documentation; no sign-in.", { registration: "none" }),
  preset("github", "GitHub", "https://api.githubcopilot.com/mcp/", "Repositories, issues and pull requests; an organization that restricts OAuth apps has to approve Yoru.", {
    registration: "own_client", needs_client_secret: true, app_client: true, setup: "Create a GitHub OAuth app (Settings, Developer settings) with callback URL http://127.0.0.1/callback.",
  }),
  preset("gmail", "Gmail", "https://gmail.googleapis.com", "The mail of your Google account: search, read, reply, send, archive.", { ...google, google: "gmail" }),
  preset("google-calendar", "Google Calendar", "https://calendar.google.com", "Your calendars and their events: read, create, change, answer invitations.", { ...google, google: "calendar" }),
  preset("slack", "Slack", "https://mcp.slack.com/mcp", "Channels, messages and people of your workspace.", {
    registration: "own_client", needs_client_secret: true, setup: "Create a Slack app of your workspace (internal or published) and give its client id and secret.",
  }),
];

let connectors: ConnectorStatus[] = [
  {
    id: "linear",
    name: "Linear",
    enabled: true,
    state: "connected",
    transport: "http",
    target: "https://mcp.linear.app/mcp",
    signed_in: true,
    tools: [
      tool("list_issues", "List issues assigned to you or matching a filter."),
      tool("get_issue", "One issue with its comments."),
      tool("create_issue", "Create an issue in a team.", true),
      tool("update_issue", "Change an issue's state, assignee or fields.", true),
    ],
  },
  {
    id: "notion",
    name: "Notion",
    enabled: true,
    state: "needs_login",
    transport: "http",
    target: "https://mcp.notion.com/mcp",
    signed_in: false,
    tools: [],
  },
];

let inbox: Notification[] = [
  { id: "n5", source: "Linear", title: "YOR-142 assigned to you", text: "Marco moved \"Inbox on the board drops the last item\" to you, due Friday.", urgency: "high", url: "https://linear.app/", at: now() - 12 * MIN, read: false, by: "linear-mine" },
  { id: "n4", source: "Yoru", title: "Good morning", text: "Two meetings today, the first at 10:30 with Giulia. Three issues are waiting on you, one due Friday.", urgency: "normal", at: now() - 3 * 60 * MIN, read: false },
  { id: "n3", source: "Calendar", title: "Dentist moved", text: "Thursday's appointment is now at 17:30.", urgency: "low", at: now() - 5 * 60 * MIN, read: true },
  { id: "n2", source: "Linear", title: "Comment on YOR-131", text: "Giulia: \"Looks good to me, merging tomorrow.\"", urgency: "normal", url: "https://linear.app/", at: now() - 26 * 60 * MIN, read: true, by: "linear-mine" },
  { id: "n1", source: "ESP32 news", title: "ESP-IDF 6.1.1 is out", text: "A fix for the USB-JTAG console dropping lines.", urgency: "low", url: "https://github.com/espressif/esp-idf/releases", at: now() - 50 * 60 * MIN, read: true, by: "esp-news" },
];

let chat: ChatLine[] = [
  { from: "you", text: "Anything new this morning?", channel: "voice", at: now() - 40 * MIN },
  {
    from: "yoru",
    text: "Marco assigned you YOR-142, due Friday, and Giulia commented on YOR-131: she'll merge it tomorrow. Nothing urgent in the rest.",
    channel: "voice",
    actions: [
      { tool: "get_inbox", ok: true, summary: "5 notifications, 2 unread" },
      { tool: "linear__list_issues", ok: true, summary: "3 issues assigned to you" },
    ],
    at: now() - 40 * MIN + 9000,
  },
];

let watchers: WatchEntry[] = [
  {
    watcher: { id: "linear-mine", name: "My Linear issues", kind: "tool", tool: "linear__list_issues", arguments: { assignee: "me", limit: 20 }, every_min: 15, enabled: true, instructions: "Only when something changes for me: assigned, commented, due soon.", max_urgency: "high" },
    state: { last_run: now() - 6 * MIN, last_error: null, told: 7 },
  },
  {
    watcher: { id: "esp-news", name: "ESP32 news", kind: "task", task: "Look on the web for news about the ESP32-S3 and ESP-IDF: releases, errata, new boards.", every_min: 720, enabled: true, instructions: "", max_urgency: "low" },
    state: { last_run: now() - 200 * MIN, last_error: null, told: 2 },
  },
  {
    watcher: { id: "notion-docs", name: "Shared Notion pages", kind: "tool", tool: "notion__search", arguments: { query: "", filter: "edited_recently" }, every_min: 60, enabled: false, instructions: "", max_urgency: "normal" },
    state: { last_run: now() - 2 * 24 * 60 * MIN, last_error: "notion needs you to sign in again", told: 0 },
  },
];

let spoken: SpokenClip[] = [
  { id: "3", at: now() - 4 * MIN, text: "[happy] Fatto! Il timer da venticinque minuti è partito.", language: "it", voice: "elevenlabs:voice/x", duration_ms: 3400 },
  { id: "2", at: now() - 50 * MIN, text: "Hai una riunione alle 14:30 con Marco.", language: "it", voice: "elevenlabs:voice/x", duration_ms: 2600 },
  { id: "1", at: now() - 26 * 60 * MIN, text: "[whispers] Good night. I'll keep watch.", language: "en", voice: "elevenlabs:voice/x", duration_ms: 2100 },
];

function tone(): ArrayBuffer {
  const n = 8000;
  const buf = new ArrayBuffer(44 + n * 2);
  const v = new DataView(buf);
  const text = (at: number, s: string) => [...s].forEach((c, i) => v.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF"), v.setUint32(4, 36 + n * 2, true), text(8, "WAVEfmt "), v.setUint32(16, 16, true);
  v.setUint16(20, 1, true), v.setUint16(22, 1, true), v.setUint32(24, 16000, true), v.setUint32(28, 32000, true);
  v.setUint16(32, 2, true), v.setUint16(34, 16, true), text(36, "data"), v.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) v.setInt16(44 + i * 2, Math.sin((i / 16000) * 2 * Math.PI * 440) * 3000, true);
  return buf;
}

let memory = `# What Yoru knows about you

## About you

- Ettore, in Milan. Works on Yoru, the firmware and the app.
- Mornings are for deep work: no meetings before 10.

## What matters to you

- Yoru: the board (ESP32-S3), the app, the host.
- Giulia and Marco: they review my work.

## Tell me about

- Issues assigned to me, and comments on mine.
- Releases of ESP-IDF and LVGL.

## Don't tell me about

- Newsletters and promotions.

## Notes
`;

const noKey = params.get("key") === "none";
const onGroq = params.get("brain") === "groq";
const groqModels = [
  { id: "openai/gpt-oss-20b", note: "GPT-OSS 20B on Groq: fast, good at tools; about 5 commands a minute free" },
  { id: "openai/gpt-oss-120b", note: "GPT-OSS 120B on Groq: smarter, a little slower; same free limits" },
  { id: "qwen/qwen3.8-27b", note: "Qwen3.8 27B on Groq (preview): good Italian" },
  { id: "llama-3.3-70b-versatile", note: "Llama 3.3 70B on Groq: good Italian, no thinking" },
];

const info: AssistantInfo = {
  settings: {
    name: "Yoru",
    provider: onGroq ? "groq" : null,
    model: "claude-opus-5-5",
    groq_model: "openai/gpt-oss-120b",
    openai_model: "gpt-6-luna",
    ollama_model: "",
    effort: "medium",
    task_effort: "low",
    web_search: true,
    think: true,
    routines: { recap: true, recap_at: "08:30", recap_weekdays_only: true, recap_task: "", quiet: true, quiet_from: "22:30", quiet_to: "07:30" },
  },
  ready: !noKey,
  brain: onGroq ? "groq" : noKey ? null : "anthropic",
  key: noKey || onGroq ? null : "saved",
  groq_key: noKey ? null : onGroq ? "saved" : null,
  groq_models: groqModels,
  openai_key: null,
  openai_models: [
    { id: "gpt-6-luna", note: "GPT-6 Luna: fast and cheap ($0.10 / $0.50 a million tokens)" },
    { id: "gpt-5.4-mini", note: "GPT-5.4 Mini: smarter, still quick ($0.75 / $4.50)" },
    { id: "gpt-5.6-terra", note: "GPT-5.6 Terra: smarter still, pricier ($2 / $12)" },
  ],
  limited: null,
  dir: "~/Library/Application Support/yoru",
  quiet_until: null,
  quiet_now: false,
};

let confirmations: Confirmation[] = [];

const timers: TimersInfo = {
  timers: [{ id: 1, kind: "timer", label: "Tea", total_s: 300, left_s: 184, state: "run", ends_at: "10:42" }],
  pomodoro: null,
  show_on: "left",
  can_speak: true,
};

const yoruSettings: YoruSettings = {
  wake_word: "Hi Yoru",
  brightness: { value: 80, min: 10, max: 100 },
  night_bright: { value: 20, min: 0, max: 100 },
  auto_dim: { value: 1, min: 0, max: 1 },
  volume: { value: 60, min: 0, max: 100 },
  touch_react: { value: 1, min: 0, max: 1 },
  motion_react: { value: 1, min: 0, max: 1 },
  sleepy_after: { value: 10, min: 1, max: 60 },
  asleep_after: { value: 30, min: 5, max: 120 },
};

const voice: VoiceInfo = {
  settings: {
    enabled: true,
    model: "large-v3-turbo-q5",
    language: "auto",
    keep_recordings: false,
    speak: true,
    follow_up: true,
    intent_model: "embeddinggemma-300m",
    turn_model: "",
    keep_warm: true,
    voices: { it: "it_IT-paola-medium", en: "en_US-amy-medium" },
    llm: false,
    llm_model: "qwen3-4b",
    personality: "cute",
    personality_custom: "",
  },
  models: [
    { id: "large-v3-turbo-q5", kind: "whisper", language: null, engine: null, bytes: 574_000_000, note: "Fast and accurate on a Mac", license: "MIT", installed: true, provider: null },
    { id: "embeddinggemma-300m", kind: "intent", language: null, engine: null, bytes: 333_590_944, note: "Tells a goodbye from a request in about 20 ms", license: "Gemma", installed: true, provider: null },
    { id: "smart-turn-v3.2", kind: "turn", language: null, engine: null, bytes: 32_411_198, note: "Hears from the voice whether you finished the sentence", license: "BSD-2-Clause", installed: false, provider: null },
    { id: "small", kind: "whisper", language: null, engine: null, bytes: 488_000_000, note: "Smaller, for older computers", license: "MIT", installed: false, provider: null },
    { id: "groq:whisper-large-v3", kind: "whisper", language: null, engine: null, bytes: 0, note: "In the cloud, very fast", license: "", installed: false, provider: "groq" },
    { id: "it_IT-paola-medium", kind: "voice", language: "it", engine: "piper", bytes: 63_000_000, note: "Paola", license: "CC-BY", installed: true, provider: null },
    { id: "en_US-amy-medium", kind: "voice", language: "en", engine: "piper", bytes: 63_000_000, note: "Amy", license: "CC-BY", installed: true, provider: null },
    { id: "elevenlabs:multilingual", kind: "voice", language: null, engine: null, bytes: 0, note: "ElevenLabs, any language", license: "", installed: false, provider: "elevenlabs" },
  ],
  stt: "ready",
  stt_error: null,
  piper: "/venv/bin/python",
  piper_install: null,
  tts: "ready",
  tts_error: null,
  llm: "off",
  llm_error: null,
  llm_server: "/opt/homebrew/bin/llama-server",
  download: null,
  providers: [
    { id: "anthropic", name: "Anthropic", note: "Claude, Yoru's brain; paid by use.", keys_url: "https://platform.claude.com/settings/keys", key_env: "ANTHROPIC_API_KEY", key: info.key },
    { id: "openai", name: "OpenAI", note: "GPT, a brain for Yoru; paid by use.", keys_url: "https://platform.openai.com/api-keys", key_env: "OPENAI_API_KEY", key: info.openai_key },
    { id: "groq", name: "Groq", note: "Free with daily limits.", keys_url: "https://console.groq.com/keys", key_env: "GROQ_API_KEY", key: info.groq_key },
    { id: "ollama", name: "Ollama", note: "Models on this computer.", keys_url: "https://ollama.com/search?c=tools", key_env: "OLLAMA_API_KEY", key: null },
    { id: "elevenlabs", name: "ElevenLabs", note: "Natural voices.", keys_url: "https://elevenlabs.io/app/settings/api-keys", key_env: "ELEVENLABS_API_KEY", key: null },
  ],
};

const accountVoices: AccountVoice[] = [
  { voice_id: "yoru1", name: "Yoru (designed)", category: "generated", description: "Soft, curious, a little sleepy" },
  { voice_id: "JBFqnCBsd6RMkjVDRZzb", name: "George", category: "premade", description: null },
  { voice_id: "EXAVITQu4vr4xnJSGQGg", name: "Sarah", category: "premade", description: null },
];

/** The brain as the core picks it: the settings', else the one with a key, Claude first. */
function resolveBrain() {
  info.brain = info.settings.provider ?? (info.key ? "anthropic" : info.openai_key ? "openai" : info.groq_key ? "groq" : null);
  const ready = { anthropic: !!info.key, openai: !!info.openai_key, groq: !!info.groq_key, ollama: !!info.settings.ollama_model };
  info.ready = info.brain ? ready[info.brain] : false;
}

const offline = params.get("board") === "off";
let onboardingDone = !params.has("setup");
const handlers = new Map<string, Handler[]>();

function emit(event: string, payload: unknown) {
  for (const h of handlers.get(event) ?? []) h(structuredClone(payload));
}

const emitInbox = () => emit("inbox", inbox);
const emitChat = () => emit("chat", chat);
const emitConfirmations = () => emit("bridge", { kind: "confirmations", cards: confirmations, target: null });
const emitConnectors = () => emit("connectors", connectors);

function addChat(line: Omit<ChatLine, "at">) {
  chat = [...chat, { ...line, at: now() }];
  emitChat();
}

/** Yoru answers after a moment; asked to send or create something, it asks first. */
async function ask(text: string) {
  addChat({ from: "you", text, channel: "text" });
  emit("bridge", { kind: "voice_state", state: "thinking" });
  await sleep(1400);
  let reply = "Nothing else new since this morning. YOR-142 is still the one due first, on Friday.";
  const actions = [{ tool: "get_inbox", ok: true, summary: "2 unread" }];
  if (/creat|crea|send|mand|scriv/i.test(text)) {
    confirmations = [...confirmations, { id: "y1", tool: "linear: create_issue", text: "Create issue \"Board inbox: swipe to dismiss\" in Yoru", detail: '{"team":"YOR","title":"Board inbox: swipe to dismiss"}', asked: now(), expires: now() + 90_000 }];
    emitConfirmations();
    while (confirmations.some((c) => c.id === "y1")) await sleep(200);
    reply = lastAnswer ? "Done: YOR-143, in Yoru's backlog." : "All right, I didn't create it.";
    actions.push({ tool: "linear__create_issue", ok: lastAnswer, summary: lastAnswer ? "created YOR-143" : "denied" });
  }
  emit("bridge", { kind: "voice_state", state: "idle" });
  addChat({ from: "yoru", text: reply, channel: "text", actions });
  return { text: reply, actions, model: info.settings.model, elapsed_ms: 1400 };
}
let lastAnswer = false;

const connector = (id: string) => connectors.find((c) => c.id === id);
function patch(id: string, change: Partial<ConnectorStatus>) {
  connectors = connectors.map((c) => (c.id === id ? { ...c, ...change } : c));
  emitConnectors();
}

async function login(id: string) {
  patch(id, { state: "connecting" });
  await sleep(1800);
  patch(id, {
    state: "connected",
    signed_in: true,
    tools: [tool("search", "Search pages and databases.", false, id), tool("fetch", "Read one page.", false, id), tool("create_page", "Create a page.", true, id)],
  });
}

const COMMANDS: Record<string, (a: Record<string, any>) => unknown> = {
  status: () => ({
    board: offline ? null : "yoru-4f2a",
    link: offline ? "searching" : "connected",
    port: offline ? null : "/dev/cu.usbmodem101",
    message: offline ? "board not found on USB" : null,
    connected_age_ms: 60_000,
    firmware: offline ? null : { state: "version", kind: "yoru", version: "0.9.2", idf: "v6.1", protocol: 2 },
    logs: ["I (1203) yoru: link up", "I (1450) voice: wake word ready"],
    voice: "idle",
    port_setting: null,
    follow_pointer: true,
  }),
  assistant_info: () => info,
  set_assistant_settings: (a) => ((info.settings = a.settings), resolveBrain(), info),
  set_do_not_disturb: (a) => ((info.quiet_until = a.minutes ? now() + a.minutes * MIN : null), (info.quiet_now = !!a.minutes), info),
  ask: (a) => ask(a.text),
  chat: () => chat,
  forget_conversation: () => ((chat = []), emitChat()),
  inbox: () => inbox,
  inbox_read: (a) => ((inbox = inbox.map((n) => (n.id === a.id ? { ...n, read: true } : n))), emitInbox(), true),
  inbox_read_all: () => ((inbox = inbox.map((n) => ({ ...n, read: true }))), emitInbox(), true),
  inbox_dismiss: (a) => ((inbox = inbox.filter((n) => n.id !== a.id)), emitInbox(), true),
  inbox_clear: () => ((inbox = []), emitInbox(), true),
  open_notification: (a) => {
    const n = inbox.find((n) => n.id === a.id);
    if (n?.url) window.open(n.url, "_blank");
    return COMMANDS.inbox_read(a);
  },
  open_web: (a) => void window.open(a.url, "_blank"),
  open_key_page: () => undefined,
  memory: () => memory,
  set_memory: (a) => void (memory = a.text),
  spoken_clips: () => spoken,
  // Half a second of a quiet tone, as a 16 kHz WAV.
  spoken_audio: () => tone(),
  delete_spoken: (a) => (spoken = spoken.filter((c) => c.id !== a.id)),
  clear_spoken: () => void (spoken = []),
  export_spoken: () => "/Users/you/Downloads/yoru-spoken-20261022-153012.zip",
  watchers: () => watchers,
  put_watcher: (a) => {
    const w = a.watcher as Watcher;
    if (!w.id) w.id = w.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
    const old = watchers.find((e) => e.watcher.id === w.id);
    watchers = old ? watchers.map((e) => (e.watcher.id === w.id ? { ...e, watcher: w } : e)) : [...watchers, { watcher: w, state: { last_run: 0, last_error: null, told: 0 } }];
    return watchers;
  },
  remove_watcher: (a) => (watchers = watchers.filter((e) => e.watcher.id !== a.id)),
  run_watcher: async (a) => {
    await sleep(1500);
    watchers = watchers.map((e) => (e.watcher.id === a.id ? { ...e, state: { ...e.state, last_run: now(), last_error: null } } : e));
    const event = { kind: "watched", id: a.id, told: 0, error: null };
    emit("schedule", event);
    return event;
  },
  recap_now: async () => {
    await sleep(2500);
    inbox = [{ id: `n${now()}`, source: "Yoru", title: "Good morning", text: "One meeting left today, at 16:00. YOR-142 is due Friday.", urgency: "normal", at: now(), read: false }, ...inbox];
    emitInbox();
    emit("schedule", { kind: "recap", error: null });
    return { text: "Recap given.", actions: [], model: info.settings.model, elapsed_ms: 2500 };
  },
  connectors: () => connectors,
  connector_presets: () => presets,
  add_connector: (a) => {
    const p = presets.find((p) => p.id === a.preset);
    const config = a.config as ConnectorConfig | null;
    const id = p?.id ?? config!.id;
    if (connector(id)) throw new Error(`a connector "${id}" exists already`);
    if (p?.registration === "own_client" && !p.app_client && !a.clientId) throw new Error(`this build of Yoru Hub has no ${p.name} client: give your own in Advanced`);
    const t = config?.transport;
    const target = p?.google ? (p.google === "gmail" ? "Gmail API" : "Google Calendar API") : (p?.url ?? (t!.type === "http" ? t!.url : t!.type === "stdio" ? t!.command : t!.service));
    const transport = p ? (p.google ? "google" : "http") : t!.type;
    const status: ConnectorStatus = { id, name: p?.name ?? config!.name, enabled: true, state: "needs_login", transport, target, signed_in: false, tools: [] };
    connectors = [...connectors, status];
    emitConnectors();
    return status;
  },
  remove_connector: (a) => ((connectors = connectors.filter((c) => c.id !== a.id)), emitConnectors()),
  set_connector_enabled: (a) => patch(a.id, { enabled: a.enabled, state: a.enabled ? (connector(a.id)?.signed_in ? "connected" : "needs_login") : "disabled" }),
  set_tool_policy: (a) => patch(a.id, { tools: connector(a.id)!.tools.map((t) => (t.name === a.tool ? { ...t, enabled: a.enabled, acts: a.acts } : t)) }),
  login_connector: (a) => login(a.id),
  cancel_login: () => undefined,
  logout_connector: (a) => patch(a.id, { signed_in: false, state: "needs_login" }),
  set_connector_secret: () => undefined,
  confirmations: () => confirmations,
  confirm: (a) => {
    lastAnswer = a.allow;
    confirmations = confirmations.filter((x) => x.id !== a.id);
    emitConfirmations();
    return true;
  },
  timers_info: () => timers,
  timer_tool: () => timers,
  dismiss_timers: () => timers,
  voice_info: () => voice,
  ollama_models: async () => (await sleep(300), ["llama3.1:8b", "qwen3:8b"]),
  set_voice: (a) => ((voice.settings = a.settings), voice),
  set_api_key: (a) => {
    const p = voice.providers.find((p) => p.id === a.provider);
    if (p) p.key = a.key ? "saved" : null;
    if (a.provider === "anthropic") info.key = a.key ? "saved" : null;
    if (a.provider === "groq") info.groq_key = a.key ? "saved" : null;
    if (a.provider === "openai") info.openai_key = a.key ? "saved" : null;
    resolveBrain();
    return voice;
  },
  check_api_key: async () => (await sleep(600), { state: "valid", error: null }),
  elevenlabs_voices: async () => (await sleep(500), accountVoices),
  test_voice: async () => {
    if (offline) throw new Error("Yoru's board isn't connected");
    await sleep(1200);
  },
  board_voice: () => ({ volume: 60, wake_sound: true }),
  yoru_settings: () => yoruSettings,
  yoru_set_setting: (a) => {
    const s = yoruSettings[a.name];
    if (typeof s === "object") s.value = a.value;
  },
  wifi_info: () => ({ transport: "auto", paired: [{ id: "yoru-4f2a", ssid: "Casa" }], board: { enabled: true, ssid: "Casa", state: "connected", ip: "192.168.1.42", rssi: -54, paired: true }, error: null }),
  wifi_scan: async () => (await sleep(900), { networks: [{ ssid: "Casa", rssi: -50, secure: true }, { ssid: "Casa-5G", rssi: -61, secure: true }, { ssid: "Ospiti", rssi: -72, secure: false }] }),
  list_ports: () => [{ name: "/dev/cu.usbmodem101", vid: 0x303a, pid: 0x1001, product: "Yoru", is_board: true }],
  firmware_info: () => ({ bundled: [{ kind: "yoru", size: 4081500, image: { version: "0.10.0", project: "yoru", idf: "v6.1", built: "Oct 19 2026 14:22:00" } }], esptool: { python: "3.12", version: "5.1" } }),
  flash_board: async () => {
    for (let p = 0; p <= 100; p += 5) {
      emit("flash", { kind: "progress", percent: p });
      await sleep(120);
    }
  },
  mcp_info: () => ({ command: "/Applications/Yoru.app/Contents/MacOS/yoru-app", args: ["--mcp"], registered: true, config_path: "~/.claude.json", hub_error: null }),
  claude_mcp: () => COMMANDS.mcp_info({}),
  ui_language: () => null,
  onboarding: () => ({ done: onboardingDone, tour_version: 0 }),
  set_onboarding: (a) => void (onboardingDone = a.done ?? onboardingDone),
  pointer_gaze: () => null,
};

export const preview = {
  async call<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
    await sleep(60);
    const run = COMMANDS[cmd];
    return structuredClone(await (run ? run(args) : undefined)) as T;
  },
  on(event: string, handler: Handler) {
    handlers.set(event, [...(handlers.get(event) ?? []), handler]);
  },
};
