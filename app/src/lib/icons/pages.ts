// Each page's icon, a 24 × 24 stroke path: the sidebar, the window bar and
// the Settings menu draw the same ones.

import type { Page } from "../store.svelte";

export const PAGE_ICONS: Record<Page, string> = {
  today: "M3 18h18M7 18a5 5 0 0 1 10 0M12 6v2.5M5.6 10.6l1.6 1.6M18.4 10.6l-1.6 1.6",
  conversation: "M5 6h14v9H10l-4 3v-3H5z",
  inbox: "M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15zM10 20.5a2 2 0 0 0 4 0",
  connectors: "M9 7V4M15 7V4M7 7h10v4a5 5 0 0 1-10 0zM12 16v4",
  tasks: "M9 6h10M9 12h10M9 18h10M4.5 5.5l1 1 2-2M4.5 11.5l1 1 2-2M4.5 17.5l1 1 2-2",
  routines: "M12 7v5l3 2M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17Z",
  memory: "M6 4h9l3 3v13H6zM9 10h6M9 14h6",
  meetings: "M12 4a3 3 0 0 1 3 3v4a3 3 0 0 1-6 0V7a3 3 0 0 1 3-3ZM6 11a6 6 0 0 0 12 0M12 17v3",
  spoken: "M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2",
  animations: "M4 6h16v12H4zM10 9.5v5l4-2.5z",
  settings: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6ZM19.4 13.5a7.6 7.6 0 0 0 0-3l2-1.6-2-3.4-2.4 1a7.5 7.5 0 0 0-2.6-1.5L14 2.5h-4l-.4 2.5A7.5 7.5 0 0 0 7 6.5l-2.4-1-2 3.4 2 1.6a7.6 7.6 0 0 0 0 3l-2 1.6 2 3.4 2.4-1a7.5 7.5 0 0 0 2.6 1.5l.4 2.5h4l.4-2.5a7.5 7.5 0 0 0 2.6-1.5l2.4 1 2-3.4z",
};

/** The pages Settings opens from its menu, not the sidebar. */
export const SETTINGS_PAGES = ["connectors", "spoken", "animations"] as const satisfies readonly Page[];
