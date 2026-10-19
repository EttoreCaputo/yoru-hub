// Times as people say them, in the app's language.

import { i18n } from "../i18n/index.svelte";

const MIN = 60_000;

/** "just now", "12 min ago", "3 h ago", "yesterday", or the date. */
export function ago(at: number, now = Date.now()): string {
  const t = i18n.t.time;
  const d = now - at;
  if (d < MIN) return t.now;
  if (d < 60 * MIN) return t.min(Math.floor(d / MIN));
  if (d < 12 * 60 * MIN) return t.hours(Math.floor(d / (60 * MIN)));
  const day = (x: number) => new Date(x).toDateString();
  if (day(at) === day(now - 24 * 60 * MIN)) return t.yesterday;
  if (day(at) === day(now)) return i18n.time(at);
  return i18n.date(at);
}

/** "4:05" or "1:02:30" from seconds. */
export function clock(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}
