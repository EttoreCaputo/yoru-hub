// The tabs in the window bar, as in Pocket: the first is the page the
// sidebar picks (Home, Memory, Settings…); after it, the things opened on
// their own, a conversation or a meeting's transcript and summary. They are
// kept on this Mac (localStorage), so the window opens on what was open.

export type Tab = { kind: "chat" | "meeting"; id: string };

const KEY = "yoru.tabs";
/** More than this and the oldest one not shown closes. */
const MAX = 12;

function read(): { list: Tab[]; active: number } {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "null");
    const list = Array.isArray(saved?.list) ? saved.list.filter((t: Tab) => (t?.kind === "chat" || t?.kind === "meeting") && typeof t.id === "string") : [];
    const active = Number.isInteger(saved?.active) && saved.active < list.length ? saved.active : -1;
    return { list, active };
  } catch {
    return { list: [], active: -1 };
  }
}

class Tabs {
  list = $state<Tab[]>(read().list);
  /** The tab shown; -1 is the page's. */
  active = $state(read().active);
  shown: Tab | null = $derived(this.list[this.active] ?? null);

  /** Show `tab`, opening it if it isn't yet. */
  open(tab: Tab) {
    let at = this.list.findIndex((t) => t.kind === tab.kind && t.id === tab.id);
    if (at < 0) {
      this.list = [...this.list, tab];
      at = this.list.length - 1;
      if (this.list.length > MAX) {
        this.list = this.list.slice(1);
        at--;
      }
    }
    this.active = at;
    this.#save();
  }

  select(at: number) {
    this.active = at < this.list.length ? at : -1;
    this.#save();
  }

  /** Back to the page's tab. */
  page() {
    this.select(-1);
  }

  /** Close tab `at`: the one next to it is shown, else the page. */
  close(at: number) {
    this.list = this.list.filter((_, i) => i !== at);
    if (this.active === at) this.active = Math.min(at, this.list.length - 1);
    else if (this.active > at) this.active--;
    this.#save();
  }

  /** Close every tab `gone` says no longer exists. */
  prune(gone: (t: Tab) => boolean) {
    for (let i = this.list.length - 1; i >= 0; i--) if (gone(this.list[i])) this.close(i);
  }

  #save() {
    try {
      localStorage.setItem(KEY, JSON.stringify({ list: this.list, active: this.active }));
    } catch {
      // Remembered for this run only.
    }
  }
}

export const tabs = new Tabs();
