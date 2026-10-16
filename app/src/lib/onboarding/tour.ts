// The tour's steps, in order. Each points at the element with
// `data-tour="<target>"`; a step without a target (or whose element isn't on
// screen, like the pet before the board says it has one) shows in the middle.
// Its texts are `onboarding.tour.steps[id]`. A step with `only` belongs to
// that board's home: DualEye's and Yoru's share the `board` and `telemetry`
// targets but not what's there.
//
// A step new in a later app version gets `since` = that tour version, and
// TOUR_VERSION goes up: people who saw the older tour are offered just those.

import type { BoardKind } from "../monitor.svelte";

export const TOUR_VERSION = 1;

export type TourId = "faces" | "telemetry" | "pet" | "timers" | "yoru" | "pills" | "settings" | "tray";
export type TourStep = { id: TourId; target?: string; only?: BoardKind; since: number };

export const TOUR: TourStep[] = [
  { id: "faces", target: "board", only: "dualeye", since: 1 },
  { id: "telemetry", target: "telemetry", only: "dualeye", since: 1 },
  { id: "pet", target: "pet", only: "dualeye", since: 1 },
  { id: "timers", target: "board", only: "dualeye", since: 1 },
  { id: "yoru", target: "board", only: "yoru", since: 1 },
  { id: "pills", target: "telemetry", only: "yoru", since: 1 },
  { id: "settings", target: "settings", since: 1 },
  { id: "tray", since: 1 },
];
