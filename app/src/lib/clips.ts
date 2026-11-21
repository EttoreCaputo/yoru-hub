// Yoru's animation clips, drawn in the app as the board draws them: the
// frames come from the firmware (yoru/tools/clips/frames.json, copied to
// firmware/yoru/clips.json by tools/fetch_firmware.sh), the drawing is
// yoru/tools/clips/preview.py's, itself a copy of main/eyes.c's layout.
// Colours and shading are simpler than the board's; shapes, positions and
// timing are the same.

type Raw = { fps: number; fields: string[]; scale: number[]; clips: Record<string, { loop: boolean; frames: number[][] }> };
export type Frame = Record<string, number>;
export type Clip = { name: string; loop: boolean; frames: Frame[]; fps: number; still: number };

// Vite reads this at build time and loads the file only when asked.
const files = import.meta.glob<Raw>("../../../firmware/yoru/clips.json", { import: "default" });

let loading: Promise<Map<string, Clip>> | null = null;

/** Every clip of the bundled firmware, by name (empty when it has none). */
export function loadClips(): Promise<Map<string, Clip>> {
  loading ??= (async () => {
    const load = Object.values(files)[0];
    const raw = load ? await load() : null;
    const out = new Map<string, Clip>();
    if (!raw) return out;
    for (const [name, c] of Object.entries(raw.clips)) {
      const frames = c.frames.map((v) => Object.fromEntries(raw.fields.map((f, j) => [f, v[j] / raw.scale[j]])));
      out.set(name, { name, loop: c.loop, frames, fps: raw.fps, still: telling(frames) });
    }
    return out;
  })();
  return loading;
}

// main/eyes.c and preview.py.
const SCREEN = 466;
const C = SCREEN / 2;
const R = 98; // LOOK_SPHERE_R
const K = 0.7; // LOOK_SCALE
const FACE_Y = 16 * K;
const NEUTRAL_W = 26;
const NEUTRAL_H = 60;
const HAND_RX = 27 * K;
const HAND_RY = 22 * K;

const PEARL = "rgb(232,228,240)";
const PEARL_SHADE = "rgb(226,221,236)";
const SHADE = "rgb(190,184,206)";
const EYE = "rgb(29,27,36)";
const HAND = "rgb(238,234,246)";

/** The effects' colour when no service is picked (FX_DEFAULT_RGB). */
export const DEFAULT_RGB = 0xffd45c;

/** The colour of a service, as the board picks it (main/errand.c, LOOKS). */
export function serviceRgb(service: string): number {
  const LOOKS: [string, number][] = [
    ["mail", 0xea4335], ["calendar", 0x4285f4], ["drive", 0x34a853], ["github", 0xe8e8ec], ["slack", 0xe01e5a],
    ["spotify", 0x1db954], ["web", 0x40c8e0], ["memory", 0xbf5af2], ["task", 0x34c759], ["timer", 0xff9f0a],
  ];
  const s = service.toLowerCase();
  return LOOKS.find(([word]) => s.includes(word))?.[1] ?? DEFAULT_RGB;
}

/** The frame that shows the clip best when it's still: the one furthest from its first. */
function telling(frames: Frame[]): number {
  const first = frames[0];
  let best = 0;
  let most = -1;
  frames.forEach((f, i) => {
    let d = 0;
    for (const k in f) d += Math.abs(f[k] - first[k]) * (Math.abs(first[k]) > 2 || ["bx", "by", "lx", "ly", "rx", "ry", "px", "py", "fx", "fy", "lex", "rex"].includes(k) ? 1 / 40 : 1);
    if (d > most) [most, best] = [d, i];
  });
  return best;
}

type Eye = { x: number; y: number; w: number; h: number; tl: number; tr: number; bottom: number };
type Layout = {
  bx: number; by: number; rx: number; ry: number;
  eyes: Eye[]; hands: [number, number, number][]; front: boolean;
  prop: [number, number, number]; glow: number; spark: number;
};

/** Where everything of frame `f` is, on the 466-pixel screen (preview.py's layout). */
function layout(f: Frame): Layout {
  const d = f.near;
  const bx = C + f.bx * K * d;
  const by = C + f.by * K * d;
  const rx = R * f.bsx * d;
  const ry = R * f.bsy * d;
  const sx = rx / R;
  const sy = ry / R;
  const a = (f.rot * Math.PI) / 180;
  const [ca, sa] = [Math.cos(a), Math.sin(a)];
  const eyes = (["l", "r"] as const).map((p) => {
    const w = NEUTRAL_W * K * f.turn * f[p + "ew"] * sx;
    const h = NEUTRAL_H * K * f[p + "eh"] * sy;
    const dx = (f[p + "ex"] * f.turn + f.fx) * K * sx;
    const dy = (FACE_Y + f.fy * K) * sy;
    const hh = Math.max(h, 2);
    return { x: bx + dx * ca - dy * sa, y: by + dx * sa + dy * ca, w, h, tl: -hh / 2 + f[p + "tl"] * hh, tr: -hh / 2 + f[p + "tr"] * hh, bottom: f[p + "bot"] };
  });
  return {
    bx, by, rx, ry, eyes,
    hands: [
      [bx + f.lx * K * d, by + f.ly * K * d, f.ls * d],
      [bx + f.rx * K * d, by + f.ry * K * d, f.rs * d],
    ],
    front: f.front > 0.5,
    prop: [bx + f.px * K * d, by + f.py * K * d, 34 * K * d * f.ps],
    glow: f.glow,
    spark: f.spark,
  };
}

const rgbCss = (rgb: number, alpha = 1) => `rgba(${rgb >> 16},${(rgb >> 8) & 255},${rgb & 255},${alpha})`;

function ellipse(g: CanvasRenderingContext2D, x: number, y: number, rx: number, ry: number) {
  g.beginPath();
  g.ellipse(x, y, Math.max(rx, 0), Math.max(ry, 0), 0, 0, Math.PI * 2);
}

/** A pill eye, cut by its top lid's slant and, for ^ ^, by the bottom lid's arch. */
function eye(g: CanvasRenderingContext2D, e: Eye) {
  if (e.w < 0.5 || e.h < 0.5) return;
  const hw = Math.max(e.w / 2, 1);
  const hh = Math.max(e.h / 2, 1);
  g.save();
  g.translate(e.x, e.y);
  g.beginPath();
  g.roundRect(-hw, -hh, hw * 2, hh * 2, Math.min(hw, hh));
  g.clip();
  if (Math.max(e.tl, e.tr) > -hh + 0.5) {
    const [x0, x1] = [-hw - 6, hw + 6];
    g.beginPath();
    g.moveTo(x0, e.tl);
    g.lineTo(x1, e.tr);
    g.lineTo(x1, hh + 10);
    g.lineTo(x0, hh + 10);
    g.closePath();
    g.clip();
  }
  if (e.bottom > 0.01) {
    const rb = 1.8 * hw;
    const cy = hh + 2 - e.bottom * (0.6 * hh + 2) + rb;
    g.beginPath();
    g.rect(-hw - 10, -hh - 10, hw * 2 + 20, hh * 2 + 20);
    g.arc(0, cy, rb, 0, Math.PI * 2);
    g.clip("evenodd");
  }
  g.fillStyle = EYE;
  g.fillRect(-hw - 1, -hh - 1, hw * 2 + 2, hh * 2 + 2);
  g.restore();
}

/** Draw frame `f` on `g`, a square of `size` pixels: the board's black disc and Yoru in it. */
export function drawFrame(g: CanvasRenderingContext2D, f: Frame, size: number, rgb = DEFAULT_RGB, mirror = false) {
  const lay = layout(f);
  g.save();
  g.clearRect(0, 0, size, size);
  g.scale(size / SCREEN, size / SCREEN);
  if (mirror) {
    g.translate(SCREEN, 0);
    g.scale(-1, 1);
  }
  ellipse(g, C, C, C, C);
  g.fillStyle = "#000";
  g.fill();
  g.clip();

  if (lay.glow > 0.01) {
    ellipse(g, lay.bx, lay.by, lay.rx * 1.35, lay.ry * 1.35);
    g.fillStyle = rgbCss(rgb, 0.55 * lay.glow);
    g.fill();
  }
  const hands = () => {
    for (const [x, y, s] of lay.hands) {
      if (s <= 0.02) continue;
      ellipse(g, x, y, HAND_RX * s, HAND_RY * s);
      g.fillStyle = HAND;
      g.fill();
      g.lineWidth = 1.5;
      g.strokeStyle = SHADE;
      g.stroke();
    }
  };
  if (!lay.front) hands();
  ellipse(g, lay.bx, lay.by, lay.rx, lay.ry);
  g.fillStyle = PEARL;
  g.fill();
  ellipse(g, lay.bx + lay.rx * 0.12, lay.by + lay.ry * 0.12, lay.rx * 0.86, lay.ry * 0.86);
  g.fillStyle = PEARL_SHADE;
  g.fill();
  if (lay.glow > 0.01) {
    ellipse(g, lay.bx, lay.by + lay.ry * 0.35, lay.rx * 0.8, lay.ry * 0.55);
    g.fillStyle = rgbCss(rgb, 0.35 * lay.glow);
    g.fill();
  }
  for (const e of lay.eyes) eye(g, e);
  const [px, py, half] = lay.prop;
  if (half > 0.5) {
    g.beginPath();
    g.roundRect(px - half, py - half, half * 2, half * 2, half * 0.35);
    g.fillStyle = rgbCss(rgb);
    g.fill();
    g.lineWidth = 1.5;
    g.strokeStyle = "#fff";
    g.stroke();
  }
  if (lay.front) hands();
  if (lay.spark > 0.01) {
    g.fillStyle = "rgb(255,250,220)";
    const n = Math.floor(3 + 10 * lay.spark);
    for (let i = 0; i < n; i++) {
      const a = i * 2.4 + lay.bx * 0.01;
      const rr = lay.rx * (1.1 + (0.25 * ((i * 37) % 7)) / 7);
      const [x, y] = [lay.bx + Math.cos(a) * rr, lay.by - Math.abs(Math.sin(a)) * rr];
      g.beginPath();
      g.moveTo(x, y - 5);
      g.lineTo(x + 5, y);
      g.lineTo(x, y + 5);
      g.lineTo(x - 5, y);
      g.fill();
    }
  }
  g.restore();
}
