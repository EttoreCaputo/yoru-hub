<!--
  The memory drawn as a graph (docs/memory.md): the people and things as
  flat round nodes in their kind's colour with their initial, their relations
  as elastic links, and each one's facts as small dots around it. A small spring
  simulation (no library: a personal memory is a few hundred nodes) keeps it
  alive: nodes float, bounce and spring back. Drag a node to stretch it (fling
  it and it flies back), drag the background to pan, scroll to zoom, click a
  node (or one of its facts) to open its card, click the background to close it.
  The first layout is worked out before it is drawn and framed once: the
  camera moves again only when the window changes size or the user asks.
-->
<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";

  type Fact = { id: number; about: number | null; section: string; text: string; to: number | null; importance: number; private: boolean };
  type Entity = { id: number; name: string; kind: string; placeholder: boolean; is_user: boolean; label: string | null };
  type Relation = { id: number; a: number; kind: string; b: number; to: number | null };
  type Graph = { entities: Entity[]; facts: Fact[]; relations: Relation[]; voices: number[] };

  let {
    graph,
    selected,
    onselect,
    youName,
    relationName,
    kindName,
    sectionName,
    inset = 0,
  }: {
    graph: Graph;
    selected: number | null;
    onselect: (id: number | null, fact?: number) => void;
    youName: (name: string) => string;
    relationName: (kind: string) => string;
    kindName: (kind: string) => string;
    sectionName: (section: string) => string;
    /** Pixels on the right a card covers: a node is centred in what's left. */
    inset?: number;
  } = $props();

  // The app's own colours (app.css), so the graph follows the theme.
  const KIND_COLOURS: Record<string, string> = {
    person: "var(--think)",
    place: "var(--done)",
    project: "var(--wait)",
    organisation: "#8b6cf0",
    app: "#e0559a",
    topic: "#d4a514",
    thing: "var(--dusk)",
  };
  const SECTION_COLOURS: Record<string, string> = {
    about: "var(--pearl-mid)",
    matters: "var(--done)",
    tell: "var(--think)",
    quiet: "var(--wrong)",
    notes: "var(--dusk)",
  };

  type Node = {
    key: string;
    x: number;
    y: number;
    vx: number;
    vy: number;
    r: number;
    /** Drawn size, sprung towards its target: nodes pop in and swell on hover. */
    s: number;
    sv: number;
    born: number;
    held: boolean;
    entity?: Entity;
    fact?: Fact;
    owner?: string;
  };
  type Link = { a: Node; b: Node; length: number; stiffness: number; label?: string; loose?: boolean; bend: number; bendV: number };

  // The simulation's state is plain (not reactive); `frame` redraws it.
  let nodes: Node[] = [];
  let links: Link[] = [];
  let byKey = new Map<string, Node>();
  let frame = $state(0);
  let raf = 0;
  let now = performance.now();
  /** Frames since the last rebuild: the first ones settle fast, then it gets bouncy. */
  let age = 0;
  const calm = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  const userId = () => graph.entities.find((e) => e.is_user)?.id ?? null;
  const factCount = (id: number) => graph.facts.filter((f) => f.to === null && (f.about ?? userId()) === id).length;

  /** Build the nodes and links from the graph, keeping the nodes already there. */
  function rebuild() {
    const old = byKey;
    const next = new Map<string, Node>();
    let order = 0;
    const make = (key: string, near: Node | undefined, spread: number, r: number): Node => {
      const was = old.get(key);
      if (was) return { ...was, r };
      const angle = Math.random() * Math.PI * 2;
      return {
        key,
        x: (near?.x ?? 0) + Math.cos(angle) * spread,
        y: (near?.y ?? 0) + Math.sin(angle) * spread,
        vx: 0,
        vy: 0,
        r,
        s: calm ? 1 : 0,
        sv: 0,
        born: now + order++ * (old.size ? 0 : 35),
        held: false,
      };
    };
    const user = userId();
    // The user first, so it blooms first and everything grows out of it.
    for (const e of [...graph.entities].sort((a, b) => Number(b.is_user) - Number(a.is_user))) {
      const key = `e${e.id}`;
      const r = e.is_user ? 24 : 14 + Math.min(8, Math.sqrt(factCount(e.id)) * 2.5);
      next.set(key, { ...make(key, undefined, e.is_user ? 0 : 60, r), entity: e });
    }
    const out: Link[] = [];
    const linked = new Set<number>();
    for (const rel of graph.relations.filter((r) => r.to === null)) {
      const a = next.get(`e${rel.a}`);
      const b = next.get(`e${rel.b}`);
      if (!a || !b) continue;
      linked.add(rel.a).add(rel.b);
      out.push({ a, b, length: 175, stiffness: 0.03, label: relationName(rel.kind), bend: 0, bendV: 0 });
    }
    // Yoru knows the rest through the user: a faint link keeps them near.
    const u = user !== null ? next.get(`e${user}`) : undefined;
    for (const e of graph.entities) {
      if (u && !e.is_user && !linked.has(e.id)) out.push({ a: u, b: next.get(`e${e.id}`)!, length: 240, stiffness: 0.008, loose: true, bend: 0, bendV: 0 });
    }
    for (const f of graph.facts.filter((f) => f.to === null)) {
      const owner = next.get(`e${f.about ?? user}`);
      if (!owner) continue;
      const key = `f${f.id}`;
      const n = { ...make(key, owner, owner.r + 18, f.importance >= 0.8 ? 5.5 : 4), fact: f, owner: owner.key };
      next.set(key, n);
      out.push({ a: owner, b: n, length: owner.r + 34, stiffness: 0.09, bend: 0, bendV: 0 });
    }
    byKey = next;
    nodes = [...next.values()];
    links = out;
    if (!old.size && nodes.length) {
      age = 0;
      touched = false;
      settle();
    }
  }

  /** The first layout, worked out before it is shown: the camera frames
      where the nodes end up once, instead of chasing them as they spread. */
  function settle() {
    for (let i = 0; i < SETTLE_STEPS; i++) step();
    fit(true);
  }
  const SETTLE_STEPS = 120;

  $effect(() => {
    void graph;
    void relationName;
    untrack(rebuild);
  });

  onMount(() => {
    const loop = (t: number) => {
      now = t;
      step();
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
  });
  onDestroy(() => cancelAnimationFrame(raf));

  /** One frame of the springs. */
  function step() {
    age++;
    // Settle quickly at first, then keep only a little friction: elastic.
    const keep = calm || age < 90 ? 0.6 : 0.86;
    const entities = nodes.filter((n) => n.entity);
    const facts = nodes.filter((n) => n.fact);
    for (let i = 0; i < entities.length; i++) {
      const a = entities[i];
      for (let j = i + 1; j < entities.length; j++) repel(a, entities[j], 9000);
      for (const f of facts) if (f.owner !== a.key) repel(a, f, 300);
    }
    const groups = new Map<string, Node[]>();
    for (const f of facts) groups.set(f.owner!, [...(groups.get(f.owner!) ?? []), f]);
    for (const group of groups.values()) for (let i = 0; i < group.length; i++) for (let j = i + 1; j < group.length; j++) repel(group[i], group[j], 110);
    for (const l of links) {
      const dx = l.b.x - l.a.x;
      const dy = l.b.y - l.a.y;
      const d = Math.hypot(dx, dy) || 1;
      const f = ((d - l.length) / d) * l.stiffness;
      l.a.vx += dx * f;
      l.a.vy += dy * f;
      l.b.vx -= dx * f;
      l.b.vy -= dy * f;
      // The link bows with how fast its ends move across it, and wobbles back.
      const across = ((l.b.vx - l.a.vx) * -dy + (l.b.vy - l.a.vy) * dx) / d;
      l.bendV += (Math.max(-45, Math.min(45, -across * 5)) - l.bend) * 0.12;
      l.bendV *= 0.78;
      l.bend += l.bendV;
    }
    for (const n of nodes) {
      const pull = n.entity?.is_user ? 0.02 : 0.0008;
      n.vx -= n.x * pull;
      n.vy -= n.y * pull;
      if (n.held) continue;
      n.vx *= keep;
      n.vy *= keep;
      const v = Math.hypot(n.vx, n.vy);
      if (v > 40) {
        n.vx *= 40 / v;
        n.vy *= 40 / v;
      }
      n.x += n.vx;
      n.y += n.vy;
    }
    collide();
    // Sizes: pop in, swell under the pointer, bounce a little either way.
    for (const n of nodes) {
      const target = now < n.born ? 0 : n.key === hover ? 1.22 : n.entity && n.entity.id === selected ? 1.08 : 1;
      n.sv += (target - n.s) * 0.16;
      n.sv *= 0.7;
      n.s += n.sv;
    }
    glide();
    frame++;
  }

  function repel(a: Node, b: Node, charge: number) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let d2 = dx * dx + dy * dy;
    if (d2 < 1) {
      dx = Math.random() - 0.5;
      dy = Math.random() - 0.5;
      d2 = 1;
    }
    const min = a.r + b.r + 6;
    const f = charge / Math.max(d2, min * min) / 6;
    const d = Math.sqrt(d2);
    a.vx -= (dx / d) * f;
    a.vy -= (dy / d) * f;
    b.vx += (dx / d) * f;
    b.vy += (dy / d) * f;
  }

  /** No two nodes overlap, whatever the forces say. */
  function collide() {
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const a = nodes[i];
        const b = nodes[j];
        if (a.fact && b.fact && a.owner !== b.owner) continue;
        const pad = a.entity && b.entity ? 48 : 4;
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        const d = Math.hypot(dx, dy) || 0.01;
        const gap = a.r + b.r + pad - d;
        if (gap <= 0) continue;
        const share = a.held ? 0 : b.held ? 1 : a.fact && !b.fact ? 1 : b.fact && !a.fact ? 0 : 0.5;
        a.x -= (dx / d) * gap * share;
        a.y -= (dy / d) * gap * share;
        b.x += (dx / d) * gap * (1 - share);
        b.y += (dy / d) * gap * (1 - share);
      }
    }
  }

  // ---- the view: pan, zoom, drag, glide ----

  let width = $state(800);
  let height = $state(540);
  let view = $state({ x: 0, y: 0, k: 1 });
  /** Where the camera is gliding to, if anywhere. */
  let goal: { x: number; y: number; k: number } | null = null;
  let svg: SVGSVGElement | undefined = $state();
  let dragging: { node?: Node; startX: number; startY: number; viewX: number; viewY: number; moved: boolean; lastX: number; lastY: number } | null = null;
  let touched = false;
  let legendHeight = $state(0);
  // A new size (the window): frame it again, unless the user took the camera.
  $effect(() => {
    void width;
    void height;
    void legendHeight;
    untrack(() => !touched && nodes.length && fit(true));
  });

  function glide() {
    if (!goal) return;
    const e = 0.1;
    view = { x: view.x + (goal.x - view.x) * e, y: view.y + (goal.y - view.y) * e, k: view.k + (goal.k - view.k) * e };
    if (Math.abs(goal.x - view.x) + Math.abs(goal.y - view.y) < 0.5 && Math.abs(goal.k - view.k) < 0.002) goal = null;
  }

  const toGraph = (ev: PointerEvent | WheelEvent) => {
    const box = svg!.getBoundingClientRect();
    return { x: (ev.clientX - box.left - width / 2 - view.x) / view.k, y: (ev.clientY - box.top - height / 2 - view.y) / view.k };
  };

  function down(ev: PointerEvent, node?: Node) {
    ev.stopPropagation();
    (ev.currentTarget as Element).setPointerCapture?.(ev.pointerId);
    goal = null;
    touched = true;
    dragging = { node, startX: ev.clientX, startY: ev.clientY, viewX: view.x, viewY: view.y, moved: false, lastX: 0, lastY: 0 };
  }

  function move(ev: PointerEvent) {
    if (!dragging) return;
    if (Math.hypot(ev.clientX - dragging.startX, ev.clientY - dragging.startY) > 3) dragging.moved = true;
    if (!dragging.moved) return;
    if (dragging.node) {
      const n = dragging.node;
      const p = toGraph(ev);
      // The speed it is moved at, for the fling when let go.
      n.vx = (p.x - n.x) * 0.6;
      n.vy = (p.y - n.y) * 0.6;
      n.x = p.x;
      n.y = p.y;
      n.held = true;
    } else {
      view = { ...view, x: dragging.viewX + ev.clientX - dragging.startX, y: dragging.viewY + ev.clientY - dragging.startY };
    }
  }

  function up() {
    const d = dragging;
    dragging = null;
    if (!d) return;
    // Let go: it flies on with its speed and springs back into place.
    if (d.node) d.node.held = false;
    if (d.moved) return;
    const id = d.node?.entity?.id ?? (d.node?.fact ? (d.node.fact.about ?? userId()) : null);
    if (id !== null && id !== undefined) {
      onselect(id, d.node?.fact?.id);
      centre(`e${id}`);
    } else if (!d.node) onselect(null);
  }

  function wheel(ev: WheelEvent) {
    ev.preventDefault();
    goal = null;
    touched = true;
    const k = Math.min(3, Math.max(0.25, view.k * Math.exp(-ev.deltaY * 0.0015)));
    const p = toGraph(ev);
    // Zoom round the pointer.
    view = { k, x: view.x - p.x * (k - view.k), y: view.y - p.y * (k - view.k) };
  }

  /** Glide so node `key` is in the middle. */
  function centre(key: string) {
    const n = byKey.get(key);
    if (n) goal = { x: -n.x * view.k - inset / 2, y: -n.y * view.k, k: view.k };
  }

  /** Glide to fit everything in view; `now`, jump there. */
  export function fit(now = false) {
    if (!nodes.length) return;
    const xs = nodes.map((n) => n.x);
    const ys = nodes.map((n) => n.y);
    const [x0, x1, y0, y1] = [Math.min(...xs) - 70, Math.max(...xs) + 70, Math.min(...ys) - 60, Math.max(...ys) + 60];
    // The legend takes the top: the graph goes under it.
    const top = legendHeight + 20;
    const k = Math.min(1.15, width / (x1 - x0), (height - top) / (y1 - y0));
    goal = { k, x: (-(x0 + x1) / 2) * k, y: (-(y0 + y1) / 2) * k + top / 2 };
    if (now) {
      view = goal;
      goal = null;
    }
  }

  // ---- what to draw ----

  let hover = $state<string | null>(null);
  const focus = $derived(hover ?? (selected !== null ? `e${selected}` : null));
  /** The nodes next to the one in focus, to light them and dim the rest. */
  const near = $derived.by(() => {
    void frame;
    if (!focus) return null;
    const set = new Set([focus]);
    for (const l of links) {
      if (l.loose) continue;
      if (l.a.key === focus) set.add(l.b.key);
      if (l.b.key === focus) set.add(l.a.key);
    }
    return set;
  });
  const hovered = $derived.by(() => {
    void frame;
    return hover ? byKey.get(hover) : undefined;
  });

  /** Where a node is drawn: where the physics put it, no motion of its own. */
  const at = (n: Node) => ({ x: n.x, y: n.y });
  /** The link's curve: its ends, and a control point bowed by its bend. */
  const curve = (l: Link) => {
    const a = at(l.a);
    const b = at(l.b);
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const d = Math.hypot(dx, dy) || 1;
    const bow = l.bend + (l.label ? d * 0.06 : 0);
    return { a, b, c: { x: (a.x + b.x) / 2 + (-dy / d) * bow, y: (a.y + b.y) / 2 + (dx / d) * bow }, d };
  };
  const bezier = (q: ReturnType<typeof curve>, t: number) => ({
    x: (1 - t) * (1 - t) * q.a.x + 2 * (1 - t) * t * q.c.x + t * t * q.b.x,
    y: (1 - t) * (1 - t) * q.a.y + 2 * (1 - t) * t * q.c.y + t * t * q.b.y,
  });

  const drawn = $derived.by(() => {
    void frame;
    const ls = links.map((l) => ({ l, q: curve(l) }));
    return { links: ls, nodes: [...nodes].sort((a, b) => (a.fact ? 0 : 1) - (b.fact ? 0 : 1)).map((n) => ({ n, p: at(n) })) };
  });

  const colourOf = (n: Node) => (n.entity ? (n.entity.is_user ? "var(--pearl)" : KIND_COLOURS[n.entity.kind] ?? KIND_COLOURS.thing) : SECTION_COLOURS[n.fact!.section] ?? "var(--dusk)");
  const initial = (e: Entity) => (e.placeholder ? "?" : ([...e.name.trim()][0] ?? "?").toUpperCase());
  const nameOf = (e: Entity) => (e.is_user ? youName(e.name) : e.name);
  const kindsShown = $derived([...new Set(graph.entities.filter((e) => !e.is_user).map((e) => e.kind))]);
  const sectionsShown = $derived([...new Set(graph.facts.filter((f) => f.to === null).map((f) => f.section))]);
  const tipAt = (n: Node) => ({ x: width / 2 + view.x + at(n).x * view.k, y: height / 2 + view.y + at(n).y * view.k });
</script>

<div class="graph" bind:clientWidth={width} bind:clientHeight={height} style:--px="{view.x}px" style:--py="{view.y}px">
  <svg
    bind:this={svg}
    {width}
    {height}
    role="application"
    aria-label="Memory graph"
    onpointerdown={(e) => down(e)}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={() => (dragging = null)}
    onwheel={wheel}
  >
    <g transform="translate({width / 2 + view.x} {height / 2 + view.y}) scale({view.k})">
      {#each drawn.links as { l, q }, i (i)}
        {@const lit = !near || (near.has(l.a.key) && near.has(l.b.key))}
        <path d="M{q.a.x} {q.a.y} Q{q.c.x} {q.c.y} {q.b.x} {q.b.y}" class="link" class:loose={l.loose} class:fact={!!l.b.fact} class:dim={!lit} class:hot={!!near && lit && !!l.label} />
      {/each}
      {#each drawn.links.filter(({ l }) => l.label) as { l, q }, i (i)}
        {@const lit = !near || (near.has(l.a.key) && near.has(l.b.key))}
        {@const m = bezier(q, 0.5)}
        <text class="rel" class:dim={!lit} x={m.x} y={m.y - 6}>{l.label}</text>
      {/each}
      {#each drawn.nodes as { n, p } (n.key)}
        {@const lit = !near || near.has(n.key)}
        <g
          class="node"
          class:dim={!lit}
          transform="translate({p.x} {p.y}) scale({Math.max(0, n.s)})"
          role="button"
          tabindex="-1"
          onpointerdown={(e) => down(e, n)}
          onpointerenter={() => (hover = n.key)}
          onpointerleave={() => (hover = null)}
        >
          {#if n.entity}
            {#if n.entity.id === selected}
              <circle r={n.r + 5} class="halo" style:stroke={colourOf(n)} />
            {/if}
            <circle r={n.r} class="body" class:placeholder={n.entity.placeholder} class:user={n.entity.is_user} style:--c={colourOf(n)} />
            <text class="initial" class:user={n.entity.is_user} style:--c={colourOf(n)} style:font-size="{Math.round(n.r * 0.85)}px" dy="0.35em">{initial(n.entity)}</text>
            {#if graph.voices.includes(n.entity.id)}<circle r="4" cx={n.r * 0.72} cy={-n.r * 0.72} class="voice" />{/if}
            <text class="name" y={n.r + 16}>{nameOf(n.entity)}</text>
            {#if n.entity.label && !n.entity.is_user}<text class="label" y={n.r + 29}>{n.entity.label}</text>{/if}
          {:else}
            <circle r={n.r} class="dot" class:private={n.fact?.private} style:fill={colourOf(n)} />
          {/if}
        </g>
      {/each}
    </g>
  </svg>

  {#if hovered?.fact}
    {@const p = tipAt(hovered)}
    <div class="tip" style:left="{p.x + 12}px" style:top="{p.y + 12}px">
      <span class="faint">{sectionName(hovered.fact.section)}</span>
      {hovered.fact.text}
    </div>
  {:else if hovered?.entity && !hovered.entity.is_user}
    {@const p = tipAt(hovered)}
    <div class="tip" style:left="{p.x + hovered.r + 12}px" style:top="{p.y - 12}px">
      <span class="faint">{kindName(hovered.entity.kind)}</span>
      {nameOf(hovered.entity)}
    </div>
  {/if}

  <div class="legend" bind:clientHeight={legendHeight}>
    {#each kindsShown as k}<span><i style:background={KIND_COLOURS[k] ?? KIND_COLOURS.thing}></i>{kindName(k)}</span>{/each}
    {#each sectionsShown as s}<span><i class="small" style:background={SECTION_COLOURS[s]}></i>{sectionName(s)}</span>{/each}
  </div>
</div>

<style>
  .graph {
    position: relative;
    height: 100%;
    min-height: 0;
    /* A faint grid of dots that pans with the graph. */
    background: radial-gradient(circle, var(--rim-2) 1px, transparent 1.5px) var(--px) var(--py) / 24px 24px, var(--night);
    overflow: hidden;
    user-select: none;
  }
  svg {
    display: block;
    cursor: grab;
    touch-action: none;
  }
  svg:active {
    cursor: grabbing;
  }
  .link {
    fill: none;
    stroke: color-mix(in srgb, var(--pearl-mid) 45%, transparent);
    stroke-width: 1.25;
    stroke-linecap: round;
    transition:
      opacity 0.2s,
      stroke 0.2s;
  }
  .link.hot {
    stroke: var(--haze);
    stroke-width: 1.75;
  }
  .link.loose {
    stroke: color-mix(in srgb, var(--pearl-mid) 30%, transparent);
    stroke-dasharray: 2 5;
  }
  .link.fact {
    stroke: color-mix(in srgb, var(--pearl-mid) 30%, transparent);
    stroke-width: 1;
  }
  .rel {
    fill: var(--dusk);
    font: 500 var(--fs-small) var(--sans);
    text-anchor: middle;
    paint-order: stroke;
    stroke: var(--night);
    stroke-width: 4px;
    pointer-events: none;
    transition: opacity 0.2s;
  }
  .node {
    cursor: pointer;
    outline: none;
    transition: opacity 0.25s;
  }
  .dim {
    opacity: 0.2;
  }
  /* Flat, like the rest of the app: a soft tint of the kind's colour, its rim, its initial. */
  .body {
    fill: color-mix(in srgb, var(--c) 14%, var(--night));
    stroke: var(--c);
    stroke-width: 1.5;
  }
  .body.user {
    fill: var(--pearl);
    stroke: none;
  }
  .body.placeholder {
    fill: var(--night);
    stroke-dasharray: 3 3;
  }
  .initial {
    fill: var(--c);
    font-family: var(--display);
    font-weight: 600;
    text-anchor: middle;
    pointer-events: none;
  }
  .initial.user {
    fill: var(--night);
  }
  .halo {
    fill: none;
    stroke-width: 1.5;
    opacity: 0.45;
  }
  .voice {
    fill: var(--done);
    stroke: var(--night);
    stroke-width: 2;
  }
  .dot {
    stroke: var(--night);
    stroke-width: 1.5;
  }
  .dot.private {
    stroke: var(--wrong);
  }
  .name {
    fill: var(--pearl);
    font: 600 var(--fs-meta) var(--sans);
    text-anchor: middle;
    paint-order: stroke;
    stroke: var(--night);
    stroke-width: 4px;
    pointer-events: none;
  }
  .label {
    fill: var(--dusk);
    font: 500 var(--fs-small) var(--sans);
    text-anchor: middle;
    paint-order: stroke;
    stroke: var(--night);
    stroke-width: 4px;
    pointer-events: none;
  }
  .tip {
    position: absolute;
    max-width: 280px;
    padding: 8px 11px;
    border-radius: var(--r-md);
    background: var(--night);
    border: 1px solid var(--rim-2);
    color: var(--pearl);
    font-size: var(--fs-meta);
    line-height: 1.35;
    pointer-events: none;
    box-shadow: var(--lift);
    animation: rise 0.18s ease-out;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
  .tip span {
    display: block;
    font-size: var(--fs-small);
    margin-bottom: 2px;
  }
  .legend {
    position: absolute;
    left: 16px;
    top: 14px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    max-width: calc(100% - 32px);
    pointer-events: none;
  }
  .legend span {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 9px 3px 8px;
    border: 1px solid var(--rim-2);
    border-radius: 999px;
    background: var(--night);
    color: var(--haze);
    font-size: var(--fs-small);
  }
  .legend i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .legend i.small {
    width: 6px;
    height: 6px;
  }
</style>
