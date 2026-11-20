// How the main window looks, kept on this Mac (localStorage): the theme,
// whether the sidebar and the Home's activity panel are open, and whether
// Settings shows its advanced options. The
// notch's window never reads it: it stays dark, it's on the menu bar.

export type Theme = "system" | "light" | "dark";
export const THEMES: Theme[] = ["system", "light", "dark"];

const KEYS = { theme: "yoru.theme", sidebar: "yoru.sidebar", activity: "yoru.activity", advanced: "yoru.advanced" } as const;

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Remembered for this run only.
  }
}

class Look {
  theme = $state<Theme>((THEMES as string[]).includes(read(KEYS.theme) ?? "") ? (read(KEYS.theme) as Theme) : "system");
  sidebar = $state(read(KEYS.sidebar) !== "closed");
  activity = $state(read(KEYS.activity) !== "closed");
  /** Settings' models, ports and MCP: hidden until asked for, in every page at once. */
  advanced = $state(read(KEYS.advanced) === "on");
  #dark = $state(true);
  /** What's on screen: the theme picked, or the Mac's own when it's "system". */
  dark = $derived(this.theme === "system" ? this.#dark : this.theme === "dark");

  /** Follows the Mac's appearance and paints the page; the main window only. */
  start() {
    const media = matchMedia("(prefers-color-scheme: dark)");
    this.#dark = media.matches;
    media.addEventListener("change", (e) => (this.#dark = e.matches));
    $effect.root(() => {
      $effect(() => {
        document.documentElement.dataset.theme = this.dark ? "dark" : "light";
      });
    });
  }

  setTheme(theme: Theme) {
    this.theme = theme;
    write(KEYS.theme, theme);
  }
  toggleSidebar() {
    this.sidebar = !this.sidebar;
    write(KEYS.sidebar, this.sidebar ? "open" : "closed");
  }
  setAdvanced(on: boolean) {
    this.advanced = on;
    write(KEYS.advanced, on ? "on" : "off");
  }
  toggleActivity() {
    this.activity = !this.activity;
    write(KEYS.activity, this.activity ? "open" : "closed");
  }
}

export const look = new Look();
