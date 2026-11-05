// Turns visible tile items into renderer input. Pure functions so they can be unit-tested.
import type { Camera } from "./camera.ts";
import type { Label, Line, RectBatch } from "./render/types.ts";
import type { RGBA, Theme } from "./render/theme.ts";
import { FLAG_GENERATED, KIND_DIR, KIND_FILE, type Item } from "./tiles.ts";

export interface Style {
  theme: Theme;
  selected: number | null;
  here: number | null;
  hover: number | null;
  pinned: ReadonlySet<number>;
  /** Fill override per item (the active lens), or null for the neutral fill. */
  lensColor: (it: Item) => RGBA | null;
  /** Items to mute (e.g. the Coupling lens mutes everything but partners). */
  muted?: (it: Item) => boolean;
}

/** Must match HEADER in indexer/crates/core/src/layout.rs. */
export const HEADER = 0.055;
/** Aggregation labels ("+ N small files") only from this many hidden files. */
export const MIN_AGGREGATE = 5;

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

const kindOrder = (k: number): number => (k === KIND_DIR ? 0 : k === KIND_FILE ? 1 : 2);

/** Painter's order: containers before contents. */
export function sortForPaint(items: Item[]): Item[] {
  return items.sort((a, b) => kindOrder(a.kind) - kindOrder(b.kind) || a.depth - b.depth || a.slot - b.slot);
}

export function buildBatch(items: Item[], style: Style, originX: number, originY: number, unit: number, prev?: RectBatch): RectBatch {
  const n = items.length;
  const geom = prev && prev.geom.length >= n * 4 ? prev.geom : new Float32Array(Math.max(16, n * 4 * 1.5) | 0);
  const colors = prev && prev.colors.length >= n * 8 ? prev.colors : new Uint8Array(Math.max(32, n * 8 * 1.5) | 0);
  const stroke = prev && prev.stroke.length >= n ? prev.stroke : new Float32Array(Math.max(4, n * 1.5) | 0);
  const t = style.theme;
  const inv = 1 / unit;
  for (let i = 0; i < n; i++) {
    const it = items[i]!;
    geom[4 * i] = (it.x - originX) * inv;
    geom[4 * i + 1] = (it.y - originY) * inv;
    geom[4 * i + 2] = it.w * inv;
    geom[4 * i + 3] = it.h * inv;
    let fill: RGBA;
    if (it.kind === KIND_DIR) fill = t.dirFill;
    else if (it.kind === KIND_FILE) fill = style.lensColor(it) ?? (it.flags & FLAG_GENERATED ? t.generatedFill : t.fileFill);
    else fill = style.lensColor(it) ?? t.symbolFill;
    if (style.muted?.(it) && it.kind !== KIND_DIR) fill = t.dirFill;
    let line: RGBA = t.line;
    let sw = 1;
    if (it.slot === style.hover) line = t.muted;
    if (style.pinned.has(it.slot)) {
      line = t.foreground;
    }
    if (it.slot === style.selected || it.slot === style.here) {
      line = t.accent;
      sw = 2;
    }
    colors.set(fill, 8 * i);
    colors.set(line, 8 * i + 4);
    stroke[i] = sw;
  }
  return { originX, originY, unit, count: n, geom, colors, stroke };
}

export interface LabelContext {
  camera: Camera;
  theme: Theme;
  measure: (text: string, size: 11) => number;
  max: number;
}

/** Fits `text` into `width` px, truncating with an ellipsis; null if fewer than 3 chars fit. */
export function fitText(text: string, width: number, measure: (t: string) => number): string | null {
  if (measure(text) <= width) return text;
  const ell = "…";
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (measure(text.slice(0, mid) + ell) <= width) lo = mid;
    else hi = mid - 1;
  }
  return lo >= 3 ? text.slice(0, lo) + ell : null;
}

/**
 * Labels appear only where they fit at 11px. Dirs label their header band, files and symbols
 * their top-left corner; dirs with aggregated small files get a muted "+ N small files" line.
 */
export function buildLabels(items: Item[], ctx: LabelContext): Label[] {
  const { camera, theme } = ctx;
  const out: Label[] = [];
  const W = camera.width;
  const H = camera.height;
  const measure = (s: string) => ctx.measure(s, 11);
  // Largest first, so when we hit the cap the important labels are kept.
  const cands: { it: Item; sx: number; sy: number; sw: number; sh: number }[] = [];
  for (const it of items) {
    const [sx, sy] = camera.toScreen(it.x, it.y);
    const sw = it.w * camera.view.scale;
    const sh = it.h * camera.view.scale;
    if (sw < 28 || sh < 14) continue;
    if (sx > W || sy > H || sx + sw < 0 || sy + sh < 0) continue;
    cands.push({ it, sx, sy, sw, sh });
  }
  cands.sort((a, b) => b.sw * b.sh - a.sw * a.sh);
  const placed: Box[] = [];
  const free = (b: Box) => !placed.some((p) => b.x < p.x + p.w && p.x < b.x + b.w && b.y < p.y + p.h && p.y < b.y + b.h);
  for (const c of cands) {
    if (out.length >= ctx.max) break;
    const { it, sx, sy, sw, sh } = c;
    const isDir = it.kind === KIND_DIR;
    const band = isDir ? Math.min(sh * HEADER, sw * 0.15) + Math.min(sw, sh) * 0.008 : sh;
    if (isDir && band < 12) continue;
    // Keep labels on screen when the rect is larger than the viewport.
    const x = Math.max(sx, 0) + 4;
    const y = Math.max(sy, 0) + (isDir ? Math.max(0, (Math.min(band, 20) - 13) / 2) + 1 : 3);
    const avail = Math.min(sx + sw, W) - x - 4;
    const text = fitText(it.label, avail, measure);
    if (!text) continue;
    const box = { x, y, w: measure(text), h: 13 };
    if (!free(box)) continue;
    placed.push(box);
    const color = isDir ? theme.muted : theme.foreground;
    out.push({ text, x, y, color, size: 11 });
    if (isDir && it.hidden >= MIN_AGGREGATE && sh > 60) {
      const extra = fitText(`+ ${it.hidden.toLocaleString("en-US")} small files`, avail, measure);
      const ey = Math.min(sy + sh, H) - 17;
      if (extra) {
        const eb = { x, y: ey, w: measure(extra), h: 13 };
        if (free(eb)) {
          placed.push(eb);
          out.push({ text: extra, x, y: ey, color: theme.muted, size: 11 });
        }
      }
    }
  }
  return out;
}

/** "You are here" marker: an accent tab on the top-left corner of the current node. */
export function hereMarker(it: Item | undefined, camera: Camera, theme: Theme): Line[] {
  if (!it) return [];
  const [sx, sy] = camera.toScreen(it.x, it.y);
  const x = Math.round(Math.max(sx, 0));
  const y = Math.round(Math.max(sy, 0));
  return [{ points: [x, y + 4, x + 10, y + 4], color: theme.accent, width: 8, style: 0 }];
}
