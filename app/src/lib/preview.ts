// A mocked Yoru for the browser preview (`npm run dev`, no Tauri): enough
// state behind every command for each page to look and act like the app.
//
//   ?setup      opens the first-run setup
//   ?board=off  no board on the desk
//   ?boards=2   a second Yoru, on WiFi
//   ?key=none   no brain's key yet (no Groq, no Anthropic)
//   ?brain=groq Yoru thinks with Groq (its key only)
//   ?mic=unknown the microphone never asked (or denied, restricted): the
//               first run's permissions step, the gate, the notch's row

import type {
  Folder,
  FolderKind,
  FolderPlace,
  FolderView,
  ActionItem,
  Action,
  AccountVoice,
  ActiveAgent,
  AssistantInfo,
  BoardClip,
  ChatLine,
  ChatTitle,
  SavedChat,
  DeviceInfo,
  Confirmation,
  ConnectorConfig,
  ConnectorStatus,
  Meeting,
  MeetingSettings,
  CalendarMeeting,
  PocketStatus,
  MeetingSources,
  Notification,
  Preset,
  SpokenClip,
  Task,
  TaskEntry,
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

// An answer written in Markdown, as models do: the chat renders it (lib/markdown.ts).
const MARKDOWN_REPLY = `For the **1.0 release**, three things are left:

1. **Firmware**: merge \`YOR-142\` and flash the boards. Giulia *reviews* it tomorrow.
2. **App**: the notch's settings, see [the issue](https://linear.app/).
3. **Docs**: update \`docs/app.md\`.

| Piece | Owner | Due |
| --- | --- | --- |
| Firmware | Marco | Friday |
| App | you | Monday |

> Ship on Friday only if the boards pass the night test.

\`\`\`mermaid
flowchart LR
  A[Merge YOR-142] --> B[Flash boards]
  B --> C{Night test}
  C -->|ok| D[Release 1.0]
  C -->|fails| A
\`\`\`

\`\`\`sh
tools/flash.sh
\`\`\`

<script>alert("not run")</script> <img src=x onerror=alert(1)>`;

let chatTitle: ChatTitle = { text: "Morning issues", by_user: false, lines: 2 };
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
  { from: "you", text: "What's left before the release?", channel: "text", at: now() - 20 * MIN },
  { from: "yoru", text: MARKDOWN_REPLY, channel: "text", at: now() - 20 * MIN + 7000 },
];

const HOUR = 60 * MIN;

// The other conversations, put aside; `currentChat` is the one above.
let currentChat = "c2";
let chatIds = 2;
let archived: SavedChat[] = [
  {
    id: "c1",
    title: { text: "Trip to Rome", by_user: false, lines: 4 },
    lines: [
      { from: "you", text: "Find me a train to Rome on Friday morning", channel: "text", at: now() - 26 * HOUR },
      { from: "yoru", text: "The 7:10 Frecciarossa gets there at 10:05, 49 euros. Want me to put it in your calendar?", channel: "text", at: now() - 26 * HOUR + 8000 },
      { from: "you", text: "Yes, please", channel: "text", at: now() - 26 * HOUR + 30000 },
      { from: "yoru", text: "Done: Friday, 7:10 to 10:05.", channel: "text", at: now() - 26 * HOUR + 36000 },
    ],
  },
  {
    id: "c0",
    title: { text: "", by_user: false, lines: 0 },
    lines: [
      { from: "you", text: "What's the weather like in Milan this weekend?", channel: "voice", at: now() - 4 * 24 * HOUR },
      { from: "yoru", text: "Sunny on Saturday, 18 degrees; rain on Sunday afternoon.", channel: "voice", at: now() - 4 * 24 * HOUR + 7000 },
    ],
  },
];
const savedCurrent = (): SavedChat => ({ id: currentChat, title: chatTitle, lines: chat });
function summarize(c: SavedChat) {
  const first = c.lines.find((l) => l.from === "you") ?? c.lines[0];
  return { id: c.id, title: c.title.text, by_user: c.title.by_user, started: c.lines[0]?.at ?? 0, updated: c.lines.at(-1)?.at ?? 0, lines: c.lines.length, preview: first?.text ?? "" };
}
function conversations() {
  const list = [...archived, ...(chat.length ? [savedCurrent()] : [])].map(summarize).sort((a, b) => b.updated - a.updated);
  return { current: currentChat, list };
}
function switchTo(c: SavedChat | null) {
  if (chat.length) archived = [...archived, savedCurrent()];
  const next = c ?? { id: `c${++chatIds}`, title: { text: "", by_user: false, lines: 0 }, lines: [] };
  currentChat = next.id;
  chat = next.lines;
  chatTitle = next.title;
  emit("chat", chat);
  emit("chat_title", chatTitle);
  emit("conversations", conversations());
}
function openChat(id: string) {
  if (id === currentChat) return;
  const c = archived.find((x) => x.id === id);
  if (!c) throw "no such conversation";
  archived = archived.filter((x) => x.id !== id);
  switchTo(c);
}

let tasks: TaskEntry[] = [
  {
    task: { id: "morning-recap", name: "Morning recap", what: "Prepare the user's morning recap: today's calendar, the mail that matters, what's due.", repeat: "daily", at: "08:30", days: [1, 2, 3, 4, 5], enabled: true, acting: "never" },
    state: { last_run: now() - 7 * HOUR, last_error: null, last_result: "Told you about two meetings and YOR-142, due Friday.", runs: 21, missed: 0 },
    next: now() + 17 * HOUR,
    running: false,
  },
  {
    task: { id: "weekly-issues", name: "Weekly issues", what: "Sum up my open GitHub issues and tell me which ones have been stuck for more than a week.", repeat: "daily", at: "09:00", days: [1], enabled: true, acting: "never" },
    state: { last_run: now() - 26 * HOUR, last_error: null, last_result: "Told you about 6 open issues, 2 stuck: YOR-131 and YOR-140.", runs: 4, missed: 0 },
    next: now() + 5 * 24 * HOUR,
    running: false,
  },
  {
    task: { id: "invoices", name: "Monthly invoices", what: "Find last month's invoices in my mail, sum them up and send the total to my accountant.", repeat: "monthly", day: 1, at: "10:00", enabled: true, acting: "ask" },
    state: { last_run: 0, last_error: null, last_result: "", runs: 0, missed: 0 },
    next: now() + 27 * 24 * HOUR,
    running: false,
  },
  {
    task: { id: "server", name: "Server status", what: "Check status.example.com and tell me only if something is down.", repeat: "hourly", every: 3, enabled: false, acting: "never" },
    state: { last_run: now() - 50 * HOUR, last_error: null, last_result: "", runs: 12, missed: 1 },
    next: null,
    running: false,
  },
];

function taskChange(change: () => void): TaskEntry[] {
  change();
  emit("tasks", tasks);
  return tasks;
}

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

const boardClips: BoardClip[] = [
  { name: "celebrate", seconds: 2.83, loop: false },
  { name: "wake_stretch", seconds: 3.1, loop: false },
  { name: "sneeze", seconds: 1.6, loop: false },
  { name: "listen_loop", seconds: 1.2, loop: true },
  { name: "search_loop", seconds: 1.8, loop: true },
];

let spoken: SpokenClip[] = [
  { id: "3", at: now() - 4 * MIN, text: "[happy] Fatto! Il timer da venticinque minuti è partito.", language: "it", voice: "elevenlabs:voice/x", duration_ms: 3400 },
  { id: "2", at: now() - 50 * MIN, text: "Hai una riunione alle 14:30 con Marco.", language: "it", voice: "elevenlabs:voice/x", duration_ms: 2600 },
  { id: "4", at: now() - 2 * MIN, text: "Domani alle nove hai la call con Giulia, poi il pomeriggio è libero.", language: "it", voice: "gemini-live:Puck", duration_ms: 4100 },
  { id: "1", at: now() - 26 * 60 * MIN, text: "[whispers] Good night. I'll keep watch.", language: "en", voice: "elevenlabs:voice/x", duration_ms: 2100 },
];

const meetingSources: MeetingSources = {
  apps: [
    { id: "us.zoom.xos", name: "zoom.us", playing: true },
    { id: "com.google.Chrome", name: "Google Chrome", playing: false },
  ],
  microphones: [
    { id: "BuiltInMicrophoneDevice", name: "MacBook Pro Microphone", default: true },
    { id: "usb-mic", name: "USB Microphone", default: false },
  ],
};

// Home's folders, as folders.json keeps them.
let folders: Folder[] = [{ id: "f1", name: "Work", order: 0, updated: 0 }];
let places: FolderPlace[] = [];
const folderView = (): FolderView => ({ folders, places: places.filter((p) => p.folder && folders.some((f) => f.id === p.folder)) });

let meetings: Meeting[] = [
  {
    id: "20261029-143000",
    title: "Weekly with Marco",
    started_at: now() - 3 * 60 * MIN,
    duration_ms: 41 * 60_000,
    state: "done",
    transcribed_at: now() - 2 * 60 * MIN,
    summarized_at: now() - 2 * 60 * MIN,
    tracks: [
      { name: "others", source: "zoom.us", kind: "others", file: "others.m4a", duration_ms: 41 * 60_000, rate: 48000, channels: 2 },
      { name: "mic", source: "MacBook Pro Microphone", kind: "mic", file: "mic.m4a", duration_ms: 41 * 60_000, rate: 48000, channels: 1 },
    ],
  },
  {
    id: "20261028-091500",
    title: null,
    started_at: now() - 30 * 60 * MIN,
    duration_ms: 7 * 60_000,
    state: "interrupted",
    transcribed_at: null,
    summarized_at: null,
    tracks: [{ name: "others", source: "System audio", kind: "others", file: "others.caf", duration_ms: 7 * 60_000, rate: 48000, channels: 2 }],
  },
];
let recording: Meeting | null = null;
// The Mac calendar's meetings around today (`meeting_calendar`): one recorded
// (the weekly), one past and not recorded, and some to come.
const today0 = new Date(new Date().setHours(0, 0, 0, 0)).getTime();
const at = (day: number, h: number, m = 0) => new Date(today0).setDate(new Date(today0).getDate() + day) + (h * 60 + m) * MIN;
const soon = Math.ceil((now() + 50 * MIN) / (30 * MIN)) * 30 * MIN;
const calendar: CalendarMeeting[] = [
  { id: "ev-weekly", title: "Weekly with Marco", start: meetings[0].started_at, end: meetings[0].started_at + 45 * MIN, call: "Zoom" },
  { id: "ev-design", title: "Design review", start: soon, end: soon + 45 * MIN, call: "Meet" },
  { id: "ev-client", title: "Client call", start: at(-1, 10), end: at(-1, 11), call: "Teams" },
  { id: "ev-roadmap", title: "Q1 roadmap", start: at(1, 11), end: at(1, 12), call: "Meet" },
  { id: "ev-giulia", title: "1:1 with Giulia", start: at(2, 15), end: at(2, 15, 30), call: "Zoom" },
  { id: "ev-sprint", title: "Sprint planning", start: at(-2, 14), end: at(-2, 15), call: "Meet" },
];
// The to-dos ticked off, as `<meeting>/<text>` (done.json in the app).
const todosDone = new Set<string>();
function todosNow() {
  return meetings
    .filter((m) => m.summarized_at)
    .flatMap((m) => {
      const s = COMMANDS.meeting_summary({ id: m.id }) as { title: string; actions: ActionItem[] } | null;
      return (s?.actions ?? []).map((item) => ({ meeting: m.id, title: m.title ?? s!.title, started_at: m.started_at, item, done: todosDone.has(`${m.id}/${item.text}`) }));
    });
}
// ?pocket=unpaired (or another phase): the Pocket recorder in that state.
const pocketDay = Date.UTC(2026, 10, 19, 9, 30);
let pocket: PocketStatus = {
  phase: (params.get("pocket") as PocketStatus["phase"]) ?? "connected",
  settings: { enabled: true, auto_sync: true, delete_after_sync: false, set_clock: true, wifi: true },
  name: "PKT01_GREY_26425893",
  battery: 86,
  firmware: "1.8.9",
  wifi_firmware: "V12",
  free_mb: 59012,
  total_mb: 59636,
  recording: false,
  recordings: [
    { date: "2026-11-19", name: "20261119143012", seconds: 1834, started_at: pocketDay + 5 * 3600e3, meeting: null },
    { date: "2026-11-19", name: "20261119093000", seconds: 642, started_at: pocketDay, meeting: null },
    { date: "2026-11-18", name: "20261118161502", seconds: 2710, started_at: pocketDay - 17 * 3600e3, meeting: "20261118-171502" },
  ],
  checking: false,
  transfer: null,
  synced_at: Date.now(),
  error: null,
};
const emitPocket = () => emit("pocket", { kind: "status", status: pocket });
/** A download, as the Pocket's thread reports it. */
function pocketDownload(names: string[]) {
  let i = 0;
  const next = () => {
    const name = names[i];
    if (!name) return void ((pocket = { ...pocket, transfer: null }), emitPocket());
    const r = pocket.recordings.find((x) => x.name === name)!;
    const total = r.seconds * 4000;
    let bytes = 0;
    const tick = setInterval(() => {
      bytes = Math.min(total, bytes + total / 8);
      pocket = { ...pocket, transfer: { name, bytes, total, wifi: r.seconds >= 15 * 60, waiting: names.slice(i + 1) } };
      emitPocket();
      if (bytes >= total) {
        clearInterval(tick);
        pocket = { ...pocket, recordings: pocket.recordings.map((x) => (x.name === name ? { ...x, meeting: `pocket-${name}` } : x)) };
        i++;
        next();
      }
    }, 250);
  };
  next();
}

let meetingSettings: MeetingSettings = { transcribe: "after", transcribe_by: "local", language: "", speakers: true, detail: "detailed", summary_style: "auto", auto_summary: true, summarize_by: "", detect: false, auto_record: [] };
let meters: ReturnType<typeof setInterval> | undefined;

function startMeeting(a: Record<string, any>): Meeting {
  const r = a.request;
  const tracks: Meeting["tracks"] = [];
  if (r.system || r.app) {
    const source = r.app ? (meetingSources.apps.find((x) => x.id === r.app)?.name ?? r.app) : "System audio";
    tracks.push({ name: "others", source, kind: "others", file: null, duration_ms: 0, rate: 48000, channels: 2 });
  }
  (r.microphones as string[]).forEach((id, i) =>
    tracks.push({ name: i ? `mic-${i + 1}` : "mic", source: meetingSources.microphones.find((m) => m.id === id)?.name ?? id, kind: "mic", file: null, duration_ms: 0, rate: 48000, channels: 1 }),
  );
  recording = { id: "20261029-170000", title: r.title, started_at: now(), duration_ms: 0, state: "recording", tracks, transcribed_at: null, summarized_at: null };
  const names = tracks.map((x) => x.name);
  meters = setInterval(() => emit("meeting", { kind: "level", levels: Object.fromEntries(names.map((n) => [n, Math.random() * 0.6])) }), 250);
  return recording;
}

function stopMeeting() {
  if (!recording) return;
  clearInterval(meters);
  const duration = now() - recording.started_at;
  const done: Meeting = {
    ...recording,
    state: "done",
    duration_ms: duration,
    tracks: recording.tracks.map((x) => ({ ...x, file: `${x.name}.m4a`, duration_ms: duration })),
  };
  recording = null;
  meetings = [done, ...meetings];
  setTimeout(() => emit("meeting", { kind: "done", meeting: done }), 600);
}

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

// The memory as a graph (docs/memory.md): made-up people and facts.
const DAY = 86_400_000;
// The voices Yoru knows (memory.db), and the notes (brain/).
const memory = {
  entities: [
    { id: 1, name: "User", aliases: [] as string[], is_user: true },
    { id: 2, name: "Sofia", aliases: ["your wife"], is_user: false },
  ],
  voices: [1, 2],
};
type PreviewNote = { path: string; kind: string; title: string; description: string; aliases: string[]; private: boolean; updated: string; lines: [string, string][]; links: string[]; text: string };
const note = (path: string, kind: string, title: string, description: string, lines: [string, string][], links: string[] = [], aliases: string[] = []): PreviewNote => ({
  path,
  kind,
  title,
  description,
  aliases,
  private: false,
  updated: "2026-11-20",
  lines,
  links,
  text: `---\ntype: ${kind}\ndescription: ${description}\naliases: [${aliases.join(", ")}]\n---\n\n# ${title}\n\n## Now\n${lines.map(([, l]) => `- ${l} [Source: chat:a#1 | 2026-11-20]`).join("\n")}\n\n## Timeline\n`,
});
let notes: PreviewNote[] = [
  note("me.md", "me", "Me", "Who the user is", [
    ["about", "Works on Yoru, in Milan"],
    ["matters", "Mornings are for deep work: no meetings before 10"],
    ["matters", "Prefers short spoken answers"],
    ["tell", "Issues assigned to me, and comments on mine"],
    ["quiet", "Newsletters and promotions"],
  ], ["people/sofia.md", "people/giulia.md", "people/marco-rossi.md", "projects/yoru.md"]),
  note("people/sofia.md", "person", "Sofia", "Sofia, your wife; loves tulips", [["about", "Your wife"], ["about", "Loves tulips"], ["about", "Birthday on 14 March"]], [], ["your wife"]),
  note("people/giulia.md", "person", "Giulia", "Giulia, a colleague who reviews your work", [["about", "Reviews your work on [[projects/yoru|Yoru]]"]], ["projects/yoru.md"]),
  note("people/marco-rossi.md", "person", "Marco Rossi", "Marco Rossi, head of design moving to Stripe in January", [["about", "Head of design at Acme, moving to Stripe in January"], ["about", "Plays the drums"]], [], ["Marco"]),
  note("projects/yoru.md", "project", "Yoru", "Yoru: the board, the app and the host", [["about", "The memory is now a folder of notes"], ["about", "Ships on Fridays"]]),
  note("topics/milan.md", "topic", "Milan", "Milan, where you live", [["about", "Lives near Navigli"]]),
];

const noKey = params.get("key") === "none";
const onGroq = params.get("brain") === "groq";
const groqModels = [
  { id: "openai/gpt-oss-20b", note: "GPT-OSS 20B on Groq: fast, good at tools; about 5 commands a minute free" },
  { id: "openai/gpt-oss-120b", note: "GPT-OSS 120B on Groq: smarter, a little slower; same free limits" },
  { id: "qwen/qwen3.8-27b", note: "Qwen3.8 27B on Groq (preview): good Italian" },
];

const info: AssistantInfo = {
  settings: {
    name: "Yoru",
    provider: onGroq ? "groq" : null,
    model: "claude-opus-5-5",
    groq_model: "openai/gpt-oss-120b",
    openai_model: "gpt-6-luna",
    google_model: "gemini-4-hydrogen",
    ollama_model: "",
    effort: "medium",
    task_effort: "low",
    web_search: true,
    think: true,
    quiet_hours: { on: true, from: "22:30", to: "07:30" },
  },
  ready: !noKey,
  brain: onGroq ? "groq" : noKey ? null : "anthropic",
  key: noKey || onGroq ? null : "saved",
  groq_key: noKey ? null : onGroq ? "saved" : null,
  groq_models: groqModels,
  openai_key: null,
  google_key: null,
  google_models: [
    { id: "gemini-3.6-flash", note: "Gemini 3.6 Flash: quick and capable; free tier, then $0.75 / $3.75" },
    { id: "gemini-4-hydrogen", note: "Gemini 4 Hydrogen: fast, made for agents; free tier, then $1.50 / $7.50" },
  ],
  tavily_key: null,
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
let agents: ActiveAgent[] = [
  { agent: "claude", title: "Show the user's words as soon as they're transcribed", project: "/Users/you/Code/yoru-hub", session: null, updated: Date.now() - 40_000, run: 1, state: "working", now: "edited AgentList.svelte" },
  { agent: "codex", title: "Bigger menu on the home page", project: "/Users/you/Sites/pub", session: "c3", updated: Date.now() - 70_000, run: null, state: "working", now: null },
  { agent: "opencode", title: "Centralise the open-data API calls", project: "/Users/you/Code/Venezia-Marea", session: "ses_4", updated: Date.now() - 300_000, run: 2, state: "done", now: null },
];

const timers: TimersInfo = {
  timers: [{ id: 1, kind: "timer", label: "Tea", total_s: 300, left_s: 184, state: "run", ends_at: "10:42" }],
  pomodoro: null,
  can_speak: true,
};

let wakeWord = "heyyoru";

const yoruSettings: YoruSettings = {
  wake_word: "Hi Yoru",
  brightness: { value: 80, min: 10, max: 100 },
  night_bright: { value: 20, min: 0, max: 100 },
  auto_dim: { value: 1, min: 0, max: 1 },
  volume: { value: 60, min: 0, max: 100 },
  wake_on: { value: 1, min: 0, max: 1 },
  wake_sound: { value: 1, min: 0, max: 1 },
  yoru_sounds: { value: 1, min: 0, max: 1 },
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
    board_follow_up: false,
    intent_model: "embeddinggemma-300m",
    turn_model: "",
    keep_warm: true,
    voices: { it: "it_IT-paola-medium", en: "en_US-amy-medium" },
    llm: false,
    llm_model: "qwen3-4b",
    live: "",
    live_model: "",
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
  live_models: [["gemini-3.8-live", "Gemini 3.8 Live: the newest, quick to answer"], ["gemini-3.1-flash-live-preview", "Gemini 3.1 Flash Live (preview)"], ["gemini-2.5-flash-native-audio-preview-12-2025", "Gemini 2.5 Flash Native Audio: the most free use a day"]],
  live_voices: [["Charon", "informative", "male"], ["Fenrir", "excitable", "male"], ["Puck", "upbeat", "male"], ["Kore", "firm", "female"], ["Zephyr", "bright", "female"]],
  download: null,
  providers: [
    { id: "anthropic", name: "Anthropic", note: "Claude, Yoru's brain; paid by use.", keys_url: "https://platform.claude.com/settings/keys", key_env: "ANTHROPIC_API_KEY", key: info.key },
    { id: "openai", name: "OpenAI", note: "GPT, a brain for Yoru; paid by use.", keys_url: "https://platform.openai.com/api-keys", key_env: "OPENAI_API_KEY", key: info.openai_key },
    { id: "google", name: "Google", note: "Gemini, a brain for Yoru; free tier or paid by use.", keys_url: "https://aistudio.google.com/apikey", key_env: "GEMINI_API_KEY", key: info.google_key },
    { id: "groq", name: "Groq", note: "Free with daily limits.", keys_url: "https://console.groq.com/keys", key_env: "GROQ_API_KEY", key: info.groq_key },
    { id: "ollama", name: "Ollama", note: "Models on this computer.", keys_url: "https://ollama.com/search?c=tools", key_env: "OLLAMA_API_KEY", key: null },
    { id: "elevenlabs", name: "ElevenLabs", note: "Natural voices.", keys_url: "https://elevenlabs.io/app/settings/api-keys", key_env: "ELEVENLABS_API_KEY", key: null },
    { id: "tavily", name: "Tavily", note: "Web search.", keys_url: "https://app.tavily.com/home", key_env: "TAVILY_API_KEY", key: null },
  ],
};

const accountVoices: AccountVoice[] = [
  { voice_id: "yoru1", name: "Yoru (designed)", category: "generated", description: "Soft, curious, a little sleepy" },
  { voice_id: "JBFqnCBsd6RMkjVDRZzb", name: "George", category: "premade", description: null },
  { voice_id: "EXAVITQu4vr4xnJSGQGg", name: "Sarah", category: "premade", description: null },
];

/** The brain as the core picks it: the settings', else the one with a key, Claude first. */
function resolveBrain() {
  info.brain = info.settings.provider ?? (info.key ? "anthropic" : info.openai_key ? "openai" : info.google_key ? "google" : info.groq_key ? "groq" : null);
  const ready = { anthropic: !!info.key, openai: !!info.openai_key, google: !!info.google_key, groq: !!info.groq_key, ollama: !!info.settings.ollama_model };
  info.ready = info.brain ? ready[info.brain] : false;
}

const offline = params.get("board") === "off";
// ?boards=2: a second Yoru, on WiFi.
const boards: DeviceInfo[] = offline
  ? []
  : [
      { target: "/dev/cu.usbmodem101", id: "7c0c5f4233e4", name: null, kind: "yoru", link: "connected", port: "/dev/cu.usbmodem101", firmware: { state: "version", kind: "yoru", version: "0.18.0", idf: "v6.1" }, primary: false, current: true },
      ...(params.get("boards") === "2"
        ? [{ target: "wifi:7c0c5f424b6c", id: "7c0c5f424b6c", name: "Camera", kind: "yoru" as const, link: "connected" as const, port: "wifi:7c0c5f424b6c", firmware: { state: "version" as const, kind: "yoru" as const, version: "0.17.0", idf: "v6.1" }, primary: false, current: false }]
        : []),
    ];
const shownBoard = () => boards.find((b) => b.current) ?? null;
let onboardingDone = !params.has("setup");
const permissions = { microphone: params.get("mic") ?? "granted", calendar: params.has("setup") ? "unknown" : "granted", system_audio: params.has("setup") ? "unknown" : "granted" };
// iCloud sync on, this Mac the doer, a fetch a minute ago.
let icloud = {
  on: true,
  audio: true,
  status: {
    running: true, entitled: true, account: "available", last_sent: Date.now() - 20_000, last_fetched: Date.now() - 70_000,
    sent: 214, received: 166, errors: [], last_error: 0, device: "08db09af6e97f26d", doer: "08db09af6e97f26d", is_doer: true,
  },
};

const handlers = new Map<string, Handler[]>();

function emit(event: string, payload: unknown) {
  for (const h of handlers.get(event) ?? []) h(structuredClone(payload));
}
// From the browser's console: `yoruEmit("notch-pointer", true)` and the like.
(globalThis as { yoruEmit?: typeof emit }).yoruEmit = emit;

const emitInbox = () => emit("inbox", inbox);
const emitChat = () => (emit("chat", chat), emit("conversations", conversations()));
const emitConfirmations = () => emit("bridge", { kind: "confirmations", cards: confirmations, target: null });
const emitConnectors = () => emit("connectors", connectors);

function addChat(line: Omit<ChatLine, "at">) {
  chat = [...chat, { ...line, at: now() }];
  emitChat();
  // Named after the first answer, as Yoru does.
  if (line.from === "yoru" && !chatTitle.text && chat.length >= 2) {
    chatTitle = { text: chat[0].text.split(/\s+/).slice(0, 4).join(" "), by_user: false, lines: chat.length };
    emit("chat_title", chatTitle);
    emit("conversations", conversations());
  }
}

/** Yoru answers after a moment; asked to send or create something, it asks first. */
async function ask(text: string) {
  addChat({ from: "you", text, channel: "text" });
  emit("bridge", { kind: "voice_state", state: "thinking" });
  await sleep(1400);
  let reply = "Nothing else new since this morning. YOR-142 is still the one due first, on Friday.";
  const actions: Action[] = [{ tool: "get_inbox", ok: true, summary: "2 unread" }];
  if (/news|notizie|novit/i.test(text)) {
    reply = "Today: the EU agreed on the new AI rules, and Rome's metro line C opens two more stations next week.";
    actions.splice(0, 1, { tool: "tavily__tavily_search", asked: "ultime notizie oggi italia mondo 14 novembre 2026", ok: true, summary: "5 results" });
    actions.push({ tool: "find_tools", ok: true, summary: "…" }, { tool: "gmail__search_threads", asked: "newsletter", ok: false, summary: "timed out" });
  }
  if (/creat|crea|send|mand|scriv/i.test(text)) {
    confirmations = [...confirmations, { id: "y1", tool: "linear: create_issue", text: "Create issue \"Board inbox: swipe to dismiss\" in Yoru", detail: '{"team":"YOR","title":"Board inbox: swipe to dismiss"}', asked: now(), expires: now() + 90_000 }];
    emitConfirmations();
    while (confirmations.some((c) => c.id === "y1")) await sleep(200);
    reply = lastAnswer ? "Done: YOR-143, in Yoru's backlog." : "All right, I didn't create it.";
    actions.push({ tool: "linear__create_issue", ok: lastAnswer, summary: lastAnswer ? "created YOR-143" : "denied" });
  }
  if (/markdown|diagram/i.test(text)) {
    // Written a piece at a time, as a stream would: partial Markdown, an open mermaid fence.
    reply = MARKDOWN_REPLY;
    addChat({ from: "yoru", text: "", channel: "text" });
    for (let n = 12; n < reply.length; n += 12) {
      chat = [...chat.slice(0, -1), { ...chat[chat.length - 1], text: reply.slice(0, n) }];
      emitChat();
      await sleep(40);
    }
    chat = chat.slice(0, -1);
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
    board: shownBoard()?.id ?? null,
    boards,
    link: offline ? "searching" : "connected",
    port: shownBoard()?.port ?? null,
    message: offline ? "board not found on USB" : null,
    connected_age_ms: 60_000,
    firmware: shownBoard()?.firmware ?? null,
    logs: ["I (1203) yoru: link up", "I (1450) voice: wake word ready"],
    voice: "idle",
    port_setting: null,
    follow_pointer: true,
    notch: false,
  }),
  assistant_info: () => info,
  set_assistant_settings: (a) => ((info.settings = a.settings), resolveBrain(), info),
  set_do_not_disturb: (a) => ((info.quiet_until = a.minutes ? now() + a.minutes * MIN : null), (info.quiet_now = !!a.minutes), info),
  ask: (a) => (a.conversation && openChat(a.conversation), ask(a.text)),
  conversations: () => conversations(),
  conversation: (a) => (a.id === currentChat ? savedCurrent() : (archived.find((c) => c.id === a.id) ?? null)),
  new_conversation: () => (chat.length && switchTo(null), currentChat),
  open_conversation: (a) => openChat(a.id),
  delete_conversation: (a) => {
    if (a.id === currentChat) {
      chat = [];
      switchTo(null);
    } else {
      archived = archived.filter((c) => c.id !== a.id);
      emit("conversations", conversations());
    }
  },
  rename_conversation: (a) => {
    const text = String(a.text).trim();
    if (a.id === currentChat) COMMANDS.set_chat_title({ text });
    else archived = archived.map((c) => (c.id === a.id ? { ...c, title: { text, by_user: !!text, lines: c.lines.length } } : c));
    emit("conversations", conversations());
  },
  chat: () => chat,
  context_use: () =>
    chat.length
      ? {
          model: "claude-opus-5-5",
          tokens: 18_420,
          window: 1_000_000,
          exact: true,
          parts: [
            { kind: "instructions", tokens: 6_210 },
            { kind: "memory", tokens: 3_480 },
            { kind: "tools", tokens: 5_930 },
            { kind: "conversation", tokens: 1_640 },
            { kind: "results", tokens: 1_160 },
          ],
        }
      : null,
  forget_conversation: () => ((chat = []), switchTo(null)),
  chat_title: () => chatTitle,
  set_chat_title: (p) => {
    const text = String((p as { text: string }).text).trim();
    chatTitle = { text, by_user: !!text, lines: chat.length };
    emit("chat_title", chatTitle);
    return chatTitle;
  },
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
  memory_graph: () => structuredClone(memory),
  notes_views: () => structuredClone(notes.filter((n) => !n.path.startsWith("archive/"))),
  note_text: (a) => notes.find((n) => n.path === a.path)?.text ?? "",
  note_write: (a) => {
    const n = notes.find((n) => n.path === a.path);
    if (n) n.text = a.content;
    return `saved ${a.path}`;
  },
  note_new: (a) => {
    const path = `${a.kind === "person" ? "people" : a.kind === "project" ? "projects" : "topics"}/${a.name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}.md`;
    notes.push({ path, kind: a.kind, title: a.name, description: a.name, aliases: [], private: false, updated: "2026-11-21", lines: [], links: [], text: `---\ntype: ${a.kind}\ndescription: ${a.name}\n---\n\n# ${a.name}\n` });
    return path;
  },
  note_archive: (a) => void (notes = notes.map((n) => (n.path === a.path ? { ...n, path: `archive/${n.path.split("/").pop()}` } : n))),
  note_changes: () => [
    { at: Date.now() - 3600_000, what: "Update people/marco-rossi.md: moved to Stripe" },
    { at: Date.now() - 86400_000 * 3, what: "Add people/marco-rossi.md: met at the weekly" },
  ],
  notes_open: () => undefined,
  has_obsidian: () => true,
  memory_forget_voice: (a) => void (memory.voices = memory.voices.filter((v) => v !== a.id)),
  yoru_clips: () => boardClips,
  yoru_play_clip: () => undefined,
  yoru_stop_clip: () => undefined,
  spoken_clips: () => spoken,
  // Half a second of a quiet tone, as a 16 kHz WAV.
  spoken_audio: () => tone(),
  delete_spoken: (a) => (spoken = spoken.filter((c) => c.id !== a.id)),
  clear_spoken: (a) => void (spoken = spoken.filter((c) => c.voice.split(":")[0] !== a.source)),
  export_spoken: (a) => `/Users/you/Downloads/yoru-spoken-${a.source}-20261022-153012.zip`,
  permissions: () => ({ ...permissions }),
  // Asking says yes, as a user would; a denied one waits for System Settings.
  request_permission: (a) => {
    const kind = a.kind as keyof typeof permissions;
    if (permissions[kind] === "unknown" || permissions[kind] === "denied") permissions[kind] = "granted";
    return permissions[kind];
  },
  open_privacy: (a) => void (permissions[a.kind as keyof typeof permissions] = "granted"),
  meeting_sources: () => meetingSources,
  meetings_list: () => meetings,
  meeting_calendar: (a) => calendar.filter((e) => e.end > a.from && e.start < a.to),
  folders: () => folderView(),
  create_folder: (a) => {
    const name = String(a.name).trim();
    if (!name) throw "a folder needs a name";
    if (folders.some((f) => f.name.toLowerCase() === name.toLowerCase())) throw `there's already a folder called ${name}`;
    folders.push({ id: `f${now()}`, name, order: folders.length, updated: now() });
    return folderView();
  },
  rename_folder: (a) => ((folders = folders.map((f) => (f.id === a.id ? { ...f, name: String(a.name).trim(), updated: now() } : f))), folderView()),
  delete_folder: (a) => {
    folders = folders.filter((f) => f.id !== a.id);
    places = places.map((p) => (p.folder === a.id ? { ...p, folder: null, updated: now() } : p));
    return folderView();
  },
  move_to_folder: (a) => {
    places = [...places.filter((p) => !(p.kind === a.kind && p.id === a.id)), { kind: a.kind as FolderKind, id: String(a.id), folder: (a.folder as string | null) ?? null, updated: now() }];
    return folderView();
  },
  meeting_active: () => recording,
  meeting_start: startMeeting,
  meeting_stop: stopMeeting,
  rename_meeting: (a) => {
    meetings = meetings.map((m) => (m.id === a.id ? { ...m, title: String(a.title).trim() || null } : m));
    return meetings.find((m) => m.id === a.id);
  },
  delete_meeting: (a) => (meetings = meetings.filter((m) => m.id !== a.id)),
  meeting_audio: () => tone(),
  icloud_status: () => icloud,
  set_icloud: (a) => void (icloud = { ...icloud, on: !!a.on }),
  set_icloud_audio: (a) => void (icloud = { ...icloud, audio: !!a.on }),
  pocket_status: () => pocket,
  pocket_settings: () => pocket.settings,
  set_pocket_settings: (a) => void ((pocket = { ...pocket, settings: a.settings as PocketStatus["settings"] }), emitPocket()),
  pocket_sync: () => pocketDownload(pocket.recordings.filter((r) => !r.meeting).map((r) => r.name)),
  pocket_download: (a) => pocketDownload(a.names as string[]),
  pocket_delete: (a) => void ((pocket = { ...pocket, recordings: pocket.recordings.filter((r) => r.name !== a.name) }), emitPocket()),
  pocket_pair: (a) => {
    pocket = { ...pocket, phase: a.on ? "pairing" : "unpaired" };
    emitPocket();
    if (a.on) setTimeout(() => ((pocket = { ...pocket, phase: "connected" }), emitPocket()), 2500);
  },
  pocket_forget: () => void ((pocket = { ...pocket, phase: "unpaired", recordings: [] }), emitPocket()),
  meeting_settings: () => meetingSettings,
  set_meeting_settings: (a) => (meetingSettings = a.settings),
  meeting_model: () => true,
  meeting_model_download: () => undefined,
  meetings_transcribing: () => [],
  meeting_transcribe: (a) => {
    setTimeout(() => emit("meeting", { kind: "progress", id: a.id, track: "others", phase: "transcribe", fraction: 0.5 }), 300);
    setTimeout(() => {
      meetings = meetings.map((m) => (m.id === a.id ? { ...m, transcribed_at: now() } : m));
      emit("meeting", { kind: "transcribed", meeting: meetings.find((m) => m.id === a.id) });
    }, 1200);
  },
  meeting_transcript: (a) =>
    a.id === "20261028-091500"
      ? {
          // A simple transcript (Settings > Intelligence > Transcribes, Simple): paragraphs, no times or speakers.
          engine: "gemini:gemini-3.5-transcribe",
          language: "it",
          timed: false,
          speakers: [],
          lines: [
            { track: "others", kind: "others", start_ms: 0, end_ms: 0, text: "Ciao a tutti, partiamo dal budget del trimestre: siamo in linea con quanto previsto.", speaker: null },
            { track: "others", kind: "others", start_ms: 0, end_ms: 0, text: "Perfetto. Allora la revisione del firmware la chiudiamo entro venerdì e io aggiorno il documento.", speaker: null },
          ],
        }
      : {
          engine: "parakeet-ultra",
          language: null,
          timed: true,
          lines: [
            { track: "others", kind: "others", start_ms: 0, end_ms: 4200, text: "Ciao, mi senti? Partiamo dalla scadenza di venerdì.", speaker: 0 },
            { track: "mic", kind: "mic", start_ms: 4600, end_ms: 9100, text: "Sì, ti sento. Per venerdì manca solo la revisione del firmware.", speaker: 0 },
            { track: "others", kind: "others", start_ms: 9800, end_ms: 15000, text: "Perfetto, allora la fai tu e io aggiorno il documento.", speaker: 0 },
            { track: "others", kind: "others", start_ms: 15400, end_ms: 19000, text: "Io intanto preparo le schede per i test.", speaker: 1 },
          ],
          speakers: [
            { key: "others/0", number: 1, name: "Marco" },
            { key: "mic/0", number: 0, name: null },
            { key: "others/1", number: 2, name: null },
          ],
        },
  meeting_export: () => "/Users/you/Downloads/Weekly with Marco.md",
  meeting_import: () => null,
  meeting_summarizers: () => ({
    brain: "OpenAI · gpt-6-luna",
    models: [
      { id: "anthropic:claude-sonnet-5-5", provider: "Claude", model: "claude-sonnet-5-5", note: "Claude Sonnet 5.5: nearly as good, cheaper" },
      { id: "openai:gpt-6-luna", provider: "OpenAI", model: "gpt-6-luna", note: "GPT-6 Luna: fast and cheap" },
      { id: "google:gemini-3.8-flash", provider: "Google", model: "gemini-3.8-flash", note: "Gemini 3.8 Flash: smarter, good with tools" },
    ],
    transcribers: [
      { id: "local", detailed: true, speakers: true, local: true, ready: true },
      { id: "google:gemini-3.8-flash", detailed: true, speakers: true, local: false, ready: true },
      { id: "google:gemini-3.5-transcribe", detailed: false, speakers: false, local: false, ready: true },
      { id: "groq:whisper-large-v3-turbo", detailed: true, speakers: false, local: false, ready: false },
      { id: "openai:whisper-1", detailed: true, speakers: false, local: false, ready: true },
    ],
  }),
  meeting_summary: (a) =>
    a.id === "20261029-143000"
      ? {
          title: "Firmware review before Friday",
          overview: "Marco and you went over Friday's deadline. Only the firmware review is left; Marco updates the document meanwhile.",
          sections: [
            { title: "Firmware", items: ["**Review:** the last open item before shipping", "**Boards:** the test boards are being prepared"] },
            { title: "Documentation", items: ["Marco keeps the release notes up to date meanwhile"] },
          ],
          decisions: ["Ship on Friday"],
          actions: [
            { text: "Review the firmware", owner: "Tu", due: "venerdì", date: "2026-10-30", kind: "task", draft: "" },
            { text: "Send Marco the test results", owner: "Tu", due: "", date: "", kind: "email", draft: "Ciao Marco, ti mando i risultati dei test sulle schede. Fammi sapere se manca qualcosa." },
            { text: "Call the board supplier", owner: "Marco", due: "giovedì alle 10", date: "2026-10-29", kind: "reminder", draft: "" },
          ],
          questions: ["Whether the documentation ships with the firmware"],
          style: "meeting",
          by: "Qwen3.5-4B-Q4_K_M",
          made_at: now(),
        }
      : null,
  meeting_summarize: () => undefined,
  meeting_todos: () => todosNow(),
  set_meeting_todo: (a) => {
    const key = `${a.id}/${a.text}`;
    if (a.done) todosDone.add(key);
    else todosDone.delete(key);
    return todosNow();
  },
  rename_speaker: (a) => {
    const t = COMMANDS.meeting_transcript(a) as { speakers: { key: string; name: string | null }[] };
    t.speakers = t.speakers.map((s) => (s.key === a.key ? { ...s, name: String(a.name).trim() || null } : s));
    return t;
  },
  watchers: () => watchers,
  tasks: () => tasks,
  put_task: (a) =>
    taskChange(() => {
      const t = a.task as Task;
      if (!t.what.trim()) throw "a task needs something to do";
      if (!t.name) t.name = t.what.slice(0, 60);
      const next = t.enabled ? now() + (t.repeat === "hourly" ? t.every * HOUR : 20 * HOUR) : null;
      if (!t.id) {
        t.id = t.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
        tasks = [...tasks, { task: t, state: { last_run: 0, last_error: null, last_result: "", runs: 0, missed: 0 }, next, running: false }];
      } else tasks = tasks.map((e) => (e.task.id === t.id ? { ...e, task: t, next } : e));
    }),
  remove_task: (a) => taskChange(() => (tasks = tasks.filter((e) => e.task.id !== a.id))),
  run_task_now: async (a) => {
    await sleep(2000);
    const line = "Done: nothing worth telling this time.";
    taskChange(() => (tasks = tasks.map((e) => (e.task.id === a.id ? { ...e, state: { ...e.state, last_run: now(), last_error: null, last_result: line, runs: e.state.runs + 1 } } : e))));
    emit("schedule", { kind: "task", id: a.id, error: null });
    return line;
  },
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
  coding_active: () => agents,
  coding_stop: (a) => {
    agents = agents.map((x) => (x.run === a.run ? { ...x, state: "stopped" as const, updated: now() } : x));
    return `stopped run ${a.run}`;
  },
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
  provider_models: async (a) => {
    await sleep(400);
    const m = (model: string, name: string, chat: boolean, stt: boolean, note = "") => ({ id: `${a.provider}:${model}`, model, name, note, chat, stt, known: !!note });
    const lists: Record<string, ReturnType<typeof m>[]> = {
      google: [
        m("gemini-3.6-flash", "Gemini 3.6 Flash", true, true, "Fast and cheap"),
        m("gemini-3.6-pro", "Gemini 3.6 Pro", true, false),
        m("gemini-3.5-flash-lite", "Gemini 3.5 Flash-Lite", true, true),
        m("gemini-3.5-transcribe-live", "Gemini Transcribe Live", false, true),
        m("gemma-4-27b-it", "Gemma 4 27B", true, false),
      ],
      openai: [m("gpt-5.2", "gpt-5.2", true, false), m("gpt-5-mini", "gpt-5-mini", true, false), m("gpt-4o-transcribe", "gpt-4o-transcribe", false, true), m("whisper-1", "whisper-1", false, true)],
      anthropic: [m("claude-opus-5-5", "Claude Opus 5.5", true, false), m("claude-sonnet-5-5", "Claude Sonnet 5.5", true, false), m("claude-haiku-5-5", "Claude Haiku 5.5", true, false)],
      groq: [m("whisper-large-v3-turbo", "whisper-large-v3-turbo", false, true), m("qwen/qwen3.8-27b", "qwen/qwen3.8-27b", true, false)],
    };
    return lists[a.provider] ?? [];
  },
  set_voice: (a) => ((voice.settings = a.settings), voice),
  set_api_key: (a) => {
    const p = voice.providers.find((p) => p.id === a.provider);
    if (p) p.key = a.key ? "saved" : null;
    if (a.provider === "anthropic") info.key = a.key ? "saved" : null;
    if (a.provider === "groq") info.groq_key = a.key ? "saved" : null;
    if (a.provider === "openai") info.openai_key = a.key ? "saved" : null;
    if (a.provider === "google") info.google_key = a.key ? "saved" : null;
    if (a.provider === "tavily") info.tavily_key = a.key ? "saved" : null;
    resolveBrain();
    return voice;
  },
  check_api_key: async () => (await sleep(600), { state: "valid", error: null }),
  elevenlabs_voices: async () => (await sleep(500), accountVoices),
  test_voice: async () => {
    if (offline) throw new Error("Yoru's board isn't connected");
    await sleep(1200);
  },
  board_voice: () => ({ volume: 60, wake_sound: true, wake_word: wakeWord, wake_words: ["heyyoru", "alexa", "hiesp"] }),
  set_board_wake_word: (a) => {
    wakeWord = a.word;
  },
  yoru_settings: () => yoruSettings,
  yoru_set_setting: (a) => {
    const s = yoruSettings[a.name];
    if (typeof s === "object") s.value = a.value;
  },
  wifi_info: () => ({ transport: "auto", paired: [{ id: "yoru-4f2a", ssid: "Casa" }], board: { enabled: true, ssid: "Casa", state: "connected", ip: "192.168.1.42", rssi: -54, paired: true }, error: null }),
  wifi_scan: async () => (await sleep(900), { networks: [{ ssid: "Casa", rssi: -50, secure: true }, { ssid: "Casa-5G", rssi: -61, secure: true }, { ssid: "Ospiti", rssi: -72, secure: false }] }),
  boards: () => boards,
  show_board: (a) => (boards.forEach((b) => (b.current = b.target === a.board)), boards),
  set_primary: (a) => (boards.forEach((b) => (b.primary = b.id === a.board)), boards),
  rename_board: (a) => (boards.forEach((b) => b.id === a.id && (b.name = a.name.trim() || null)), boards),
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
