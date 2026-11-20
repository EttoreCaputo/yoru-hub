// Each page's icon (icons.ts): the sidebar, the window bar
// and the Settings menu draw the same ones.

import type { Page } from "../store.svelte";
import type { IconName } from "./icons";

export const PAGE_ICONS: Record<Page, IconName> = {
  home: "house",
  inbox: "bell",
  connectors: "plug",
  tasks: "list-checks",
  routines: "alarm-clock",
  memory: "brain",
  meetings: "users",
  todos: "list-todo",
  spoken: "audio-lines",
  animations: "eye",
  settings: "settings",
};

/** The pages Settings opens from its menu, not the sidebar. */
export const SETTINGS_PAGES = ["tasks", "routines", "connectors", "spoken", "animations"] as const satisfies readonly Page[];
