// The Meetings page's state, shared by the page (Meetings.svelte) and the
// panel on the window's right (MeetingPanel.svelte): the week or day shown,
// the calendar's meetings in it, the recordings laid on the same hours, and
// the one picked.

import { call, why } from "./api";
import { store } from "./store.svelte";
import type { CalendarMeeting, Meeting } from "./types";

const MIN = 60_000;
const DAY = 24 * 60 * MIN;
/** A recording that starts this early still belongs to the meeting. */
const EARLY = 10 * MIN;
/** The calendar is read again this often while the page is open. */
const REFRESH = 5 * MIN;

/** How a block on the calendar stands. */
export type BlockState = "live" | "work" | "done" | "recorded" | "cal" | "miss";

/** A meeting on the calendar: the calendar's, a recording, or both at once. */
export type Block = {
  key: string;
  start: number;
  end: number;
  /** Empty for a recording without a title (the page names it by its time). */
  title: string;
  event?: CalendarMeeting;
  meeting?: Meeting;
  state: BlockState;
};

/** What the panel shows: a calendar meeting, a recording, or nothing (today). */
export type Picked = { event?: string; meeting?: string } | null;

export function startOfDay(at: number): number {
  const d = new Date(at);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Monday of `at`'s week. */
export function startOfWeek(at: number): number {
  const d = new Date(startOfDay(at));
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return d.getTime();
}

/** The day `n` days after `day`, at its midnight (DST-safe). */
export function addDays(day: number, n: number): number {
  const d = new Date(day);
  d.setDate(d.getDate() + n);
  return d.getTime();
}

export function meetingState(m: Meeting): BlockState {
  if (m.state === "recording") return "live";
  if (m.id in store.transcribing || m.id in store.summarizing) return "work";
  if (m.summarized_at || m.transcribed_at) return "done";
  return "recorded";
}

/** The calendar's meetings with the recordings made during them, and the
 *  other recordings on their own, in time order. */
export function blocks(events: CalendarMeeting[], meetings: Meeting[], now: number): Block[] {
  const used = new Set<string>();
  const out: Block[] = [];
  const sameDay = (a: number, b: number) => startOfDay(a) === startOfDay(b);
  for (const e of events) {
    const free = meetings.filter((m) => !used.has(m.id));
    const m =
      free.find((m) => m.started_at >= e.start - EARLY && m.started_at <= e.end) ??
      free.find((m) => !!m.title && m.title === e.title && sameDay(m.started_at, e.start));
    if (m) used.add(m.id);
    out.push({ key: `e:${e.id}`, start: e.start, end: e.end, title: m?.title || e.title, event: e, meeting: m, state: m ? meetingState(m) : e.end < now ? "miss" : "cal" });
  }
  for (const m of meetings) {
    if (used.has(m.id)) continue;
    // Short ones still get a block that can be read and clicked.
    const end = m.state === "recording" ? Math.max(now, m.started_at + 15 * MIN) : m.started_at + Math.max(m.duration_ms, 15 * MIN);
    out.push({ key: `m:${m.id}`, start: m.started_at, end, title: m.title ?? "", meeting: m, state: meetingState(m) });
  }
  return out.sort((a, b) => a.start - b.start);
}

class MeetingsPage {
  view = $state<"week" | "day">("week");
  /** The first day shown: a Monday in the week view. */
  from = $state(startOfWeek(Date.now()));
  /** The past meetings' list instead of the calendar. */
  past = $state(false);
  picked = $state<Picked>(null);
  events = $state<CalendarMeeting[]>([]);
  /** Why the calendar can't be read: `denied`, or another reason. */
  calendarError = $state("");
  /** The record dialog: to record now, or only to pick the sources. */
  dialog = $state<{ mode: "record" | "sources"; title?: string } | null>(null);

  days = $derived(this.view === "day" ? 1 : 7);
  to = $derived(addDays(this.from, this.days));
  /** The recording that runs is in the store on its own until it ends. */
  all = $derived.by(() => {
    const r = store.recording;
    const meetings = r && !store.meetings.some((m) => m.id === r.id) ? [r, ...store.meetings] : store.meetings;
    return blocks(this.events, meetings, store.now);
  });
  /** What's picked, looked up again so it follows the recording's state. */
  block = $derived.by(() => {
    const p = this.picked;
    if (!p) return null;
    return this.all.find((b) => (p.event && b.event?.id === p.event) || (p.meeting && b.meeting?.id === p.meeting)) ?? null;
  });

  #read: { from: number; to: number; at: number } | null = null;

  /** Reads the calendar for what's shown and for today (the panel's), unless just read. */
  async readCalendar(force = false) {
    const today = startOfDay(Date.now());
    const from = Math.min(this.from, today);
    const to = Math.max(this.to, today + DAY);
    const r = this.#read;
    if (!force && r && r.from <= from && r.to >= to && Date.now() - r.at < REFRESH) return;
    try {
      this.events = await call<CalendarMeeting[]>("meeting_calendar", { from, to });
      this.calendarError = "";
      this.#read = { from, to, at: Date.now() };
    } catch (e) {
      this.calendarError = why(e);
    }
  }

  step(n: number) {
    this.from = addDays(this.from, n * this.days);
    void this.readCalendar();
  }

  today() {
    this.from = this.view === "day" ? startOfDay(Date.now()) : startOfWeek(Date.now());
    void this.readCalendar();
  }

  show(view: "week" | "day") {
    if (view === this.view) return;
    const today = startOfDay(Date.now());
    // Day view opens on today if it's in the week shown, else on the week's Monday.
    this.from = view === "day" ? (today >= this.from && today < this.to ? today : this.from) : startOfWeek(this.from);
    this.view = view;
    void this.readCalendar();
  }

  pick(b: Block | null) {
    this.picked = b ? { event: b.event?.id, meeting: b.meeting?.id } : null;
  }

  /** Marks a calendar meeting to be recorded without asking, or not. Turning
   *  one on turns the detection on too: it's what notices the call. */
  async setAuto(event: CalendarMeeting, on: boolean) {
    const s = await call<import("./types").MeetingSettings>("meeting_settings");
    const today = startOfDay(Date.now());
    // The ones already past are dropped as they're no longer needed.
    const keep = new Set(this.events.filter((e) => e.end >= today).map((e) => e.id));
    const ids = s.auto_record.filter((id) => id !== event.id && (keep.has(id) || !this.events.some((e) => e.id === id)));
    if (on) ids.push(event.id);
    await call("set_meeting_settings", { settings: { ...s, auto_record: ids, detect: on ? true : s.detect } });
    this.auto = ids;
    this.detect = on ? true : s.detect;
  }

  /** The calendar meetings marked, and whether the detection is on. */
  auto = $state<string[]>([]);
  detect = $state(false);

  async readSettings() {
    try {
      const s = await call<import("./types").MeetingSettings>("meeting_settings");
      this.auto = s.auto_record ?? [];
      this.detect = s.detect;
    } catch {
      // The panel then shows every meeting as not marked.
    }
  }
}

export const page = new MeetingsPage();
