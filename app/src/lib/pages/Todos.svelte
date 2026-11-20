<!--
  To-do (docs/meetings.md): every summarized meeting's things to do in one
  list, grouped by the meeting they came from (its title opens the meeting's
  tab). Each can be ticked off, which is kept beside the meeting's summary;
  the open ones are counted in the sidebar. A day already gone shows as late.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { store } from "../store.svelte";
  import { tabs } from "../tabs.svelte";
  import type { ActionItem, Todo } from "../types";
  import Icon from "../ui/Icon.svelte";

  const t = $derived(i18n.t.todos);
  const tm = $derived(i18n.t.meetings);

  type Filter = "open" | "done" | "all";
  let filter = $state<Filter>("open");
  let query = $state("");
  let drafting = $state<Todo | null>(null);
  let copied = $state<Todo | null>(null);
  let error = $state("");

  $effect(() => {
    void store.refreshTodos();
  });

  /** Today as `YYYY-MM-DD`, in local time, to compare with the to-dos' days. */
  const today = $derived.by(() => {
    const d = new Date(store.now);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  });
  const late = (x: Todo) => !x.done && !!x.item.date && x.item.date < today;

  const words = $derived(query.trim().toLowerCase().split(/\s+/).filter(Boolean));
  const shown = $derived(
    store.todos.filter(
      (x) =>
        (filter === "all" || (filter === "done") === x.done) &&
        words.every((w) => `${x.item.text} ${x.item.owner} ${x.item.due} ${x.title}`.toLowerCase().includes(w)),
    ),
  );
  /** The shown to-dos by meeting, in the list's order (newest meeting first). */
  const groups = $derived.by(() => {
    const out: { meeting: string; title: string; started_at: number; items: Todo[] }[] = [];
    for (const x of shown) {
      const last = out.at(-1);
      if (last?.meeting === x.meeting) last.items.push(x);
      else out.push({ meeting: x.meeting, title: x.title, started_at: x.started_at, items: [x] });
    }
    return out;
  });
  const counts = $derived({ open: store.openTodos, done: store.todos.length - store.openTodos, all: store.todos.length });

  /** When it's due: its day, else what was said. */
  const dueOf = (a: ActionItem) => {
    const [y, mo, d] = (a.date ?? "").split("-").map(Number);
    if (!(y && mo && d)) return a.due;
    return a.date === today ? t.today : i18n.date(new Date(y, mo - 1, d).getTime(), { weekday: "short", day: "numeric", month: "short" });
  };
  const kindOf = (a: ActionItem) => (a.kind in tm.kinds ? a.kind : "task") as keyof typeof tm.kinds;
  const same = (a: Todo | null, b: Todo) => !!a && a.meeting === b.meeting && a.item.text === b.item.text;

  async function tick(x: Todo, done: boolean) {
    error = "";
    try {
      await store.setTodo(x, done);
    } catch (e) {
      error = String(e);
      void store.refreshTodos();
    }
  }
  async function copy(x: Todo) {
    await navigator.clipboard.writeText(x.item.draft).catch(() => {});
    copied = x;
    setTimeout(() => same(copied, x) && (copied = null), 1500);
  }
  const openMeeting = (id: string) => tabs.open({ kind: "meeting", id });
</script>

<div class="page">
  <div class="page-head">
    <div>
      <h1>{t.title}</h1>
      <p class="lede">{t.lede}</p>
    </div>
  </div>

  {#if store.todos.length}
    <div class="bar">
      <div class="seg" role="tablist" aria-label={t.title}>
        {#each ["open", "done", "all"] as const as f (f)}
          <button role="tab" class:on={filter === f} aria-selected={filter === f} onclick={() => (filter = f)}>{t[f]}<span class="n">{counts[f]}</span></button>
        {/each}
      </div>
      <label class="search">
        <Icon name="search" size={16} />
        <input type="search" bind:value={query} placeholder={t.search} aria-label={t.search} />
      </label>
    </div>
    {#if error}<p class="error-text">{error}</p>{/if}

    {#each groups as g (g.meeting)}
      <section class="group">
        <button class="from" onclick={() => openMeeting(g.meeting)} title={t.openMeeting}>
          <Icon name="file-text" size={15} />
          <b>{g.title || tm.untitled(i18n.date(g.started_at))}</b>
          <span class="faint">{i18n.date(g.started_at, { weekday: "short", day: "numeric", month: "short" })}</span>
          <Icon name="chevron-right" size={14} />
        </button>
        <ul class="todo">
          {#each g.items as x (x.item.text)}
            {@const due = dueOf(x.item)}
            <li class:done={x.done}>
              <div class="do">
                <input type="checkbox" checked={x.done} aria-label={x.done ? t.reopen : t.markDone} onchange={(e) => tick(x, e.currentTarget.checked)} />
                <span class="what">
                  {x.item.text}
                  {#if x.item.owner || due}
                    <span class="meta">
                      {#if x.item.owner}<span>{x.item.owner}</span>{/if}
                      {#if due}<span class:late={late(x)}>{late(x) ? t.late(due) : due}</span>{/if}
                    </span>
                  {/if}
                </span>
                <span class="kind {kindOf(x.item)}">{tm.kinds[kindOf(x.item)]}</span>
              </div>
              {#if x.item.draft}
                <div class="draft-row">
                  <button class="btn link small" aria-expanded={same(drafting, x)} onclick={() => (drafting = same(drafting, x) ? null : x)}>{tm.showDraft}</button>
                </div>
                {#if same(drafting, x)}
                  <div class="draft well">
                    <p>{x.item.draft}</p>
                    <button class="btn small" onclick={() => copy(x)}>{same(copied, x) ? tm.copied : tm.copyDraft}</button>
                  </div>
                {/if}
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {:else}
      <div class="panel empty"><b>{words.length ? t.noMatch : filter === "open" ? t.allDone : t.noneDone}</b></div>
    {/each}
  {:else}
    <div class="panel empty">
      <b>{t.none}</b>
      <p>{t.noneHint}</p>
      <button class="btn" onclick={() => store.go("meetings")}>{t.toMeetings}</button>
    </div>
  {/if}
</div>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    margin-bottom: 22px;
  }
  .seg .n {
    margin-left: 6px;
    color: var(--dusk);
    font-variant-numeric: tabular-nums;
  }
  .search {
    flex: 1 1 220px;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 8px 0 14px;
    border: 1px solid var(--rim-2);
    border-radius: 999px;
    background: var(--field);
    color: var(--dusk);
    transition: border-color 160ms;
  }
  .search:focus-within {
    border-color: var(--pearl-deep);
  }
  .search input {
    flex: 1;
    min-width: 0;
    min-height: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: none;
    box-shadow: none;
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  .group + .group {
    margin-top: 26px;
  }
  .from {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 100%;
    margin: 0 0 10px -8px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    color: var(--haze);
    cursor: pointer;
    transition: background 160ms;
  }
  .from b {
    color: var(--pearl);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .from:hover {
    background: var(--hover);
  }
  .todo {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--rim);
    border-radius: var(--r-lg);
    background: var(--ink);
  }
  .todo li {
    padding: 12px 14px;
  }
  .todo li + li {
    border-top: 1px solid var(--rim);
  }
  .do {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  /* A round tick in the app's colours, not the system's grey square. */
  .do input {
    appearance: none;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    min-height: 0;
    margin: 2px 0 0;
    padding: 0;
    border: 1.5px solid var(--rim-2);
    border-radius: 50%;
    background: var(--field);
    cursor: pointer;
    transition:
      border-color 160ms,
      background 160ms;
  }
  .do input:hover {
    border-color: var(--pearl-deep);
  }
  .do input:checked {
    border-color: var(--done);
    background: var(--done);
  }
  .do input:checked::after {
    content: "";
    width: 5px;
    height: 9px;
    margin-top: -2px;
    border: solid var(--night);
    border-width: 0 2px 2px 0;
    transform: rotate(45deg);
  }
  .what {
    flex: 1;
    min-width: 0;
    color: var(--pearl);
    user-select: text;
  }
  li.done .what {
    color: var(--dusk);
    text-decoration: line-through;
    text-decoration-color: var(--rim-2);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    margin-top: 3px;
    color: var(--haze);
    font-size: var(--fs-small);
  }
  .meta .late {
    color: var(--wrong);
    font-weight: 500;
  }
  .kind {
    flex-shrink: 0;
    min-width: 76px;
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid var(--rim);
    color: var(--haze);
    font-size: var(--fs-small);
    text-align: center;
  }
  .kind.reminder {
    color: var(--wait);
    border-color: color-mix(in srgb, var(--wait) 40%, transparent);
  }
  .kind.message {
    color: var(--done);
    border-color: color-mix(in srgb, var(--done) 40%, transparent);
  }
  .kind.email {
    color: var(--think);
    border-color: color-mix(in srgb, var(--think) 40%, transparent);
  }
  .draft-row {
    margin: 4px 0 0 29px;
  }
  .draft {
    display: grid;
    justify-items: start;
    gap: 8px;
    margin: 8px 0 2px 29px;
    padding: 10px 12px;
  }
  .draft p {
    white-space: pre-wrap;
    user-select: text;
    color: var(--pearl);
  }
  .empty .btn {
    margin-top: 14px;
  }
</style>
