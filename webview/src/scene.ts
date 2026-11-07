// Turns visible tile items into renderer input. Pure functions so they can be unit-tested.
import type { Camera } from "./camera.ts";
import type { Label, Line, RectBatch } from "./render/types.ts";
import type { RGBA, Theme } from "./render/theme.ts";
import { FLAG_DATA, FLAG_GENERATED, KIND_DIR, KIND_FILE, type DecodedTile, type Item } from "./tiles.ts";

/** What decides fills: the theme and the active lens. Selection is drawn as an overlay. */
export interface BaseStyle {
  theme: Theme;
  /** Fill override per item (the active lens), or null for the neutral fill. */
  lensColor: (it: Item) => RGBA | null;
  /** Items to mute (e.g. the Coupling lens mutes everything but partners). */
  muted?: (it: Item) => boolean;
}

export interface Outline {
  it: Item;
  color: RGBA;
  width: number;
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

/** Uniform grid of placed label boxes: collision checks touch only nearby cells. */
export class BoxGrid {
  private readonly cells = new Map<number, Box[]>();
  private readonly size: number;
  constructor(size: number) {
    this.size = size;
  }
  private keys(b: Box): number[] {
    const out: number[] = [];
    const x0 = Math.floor(b.x / this.size);
    const x1 = Math.floor((b.x + b.w) / this.size);
    const y0 = Math.floor(b.y / this.size);
    const y1 = Math.floor((b.y + b.h) / this.size);
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) out.push((y + 4096) * 8192 + x + 4096);
    return out;
  }
  free(b: Box): boolean {
    for (const k of this.keys(b)) {
      for (const p of this.cells.get(k) ?? []) {
        if (b.x < p.x + p.w && p.x < b.x + b.w && b.y < p.y + p.h && p.y < b.y + b.h) return false;
      }
    }
    return true;
  }
  add(b: Box): void {
    for (const k of this.keys(b)) {
      let c = this.cells.get(k);
      if (!c) this.cells.set(k, (c = []));
      c.push(b);
    }
  }
}

/** Memoises text widths; labels repeat from frame to frame. Clear when the font changes. */
export class WidthCache {
  private readonly map = new Map<string, number>();
  private readonly measure: (t: string) => number;
  constructor(measure: (t: string) => number) {
    this.measure = measure;
  }
  width(t: string): number {
    let w = this.map.get(t);
    if (w === undefined) {
      if (this.map.size > 50_000) this.map.clear();
      w = this.measure(t);
      this.map.set(t, w);
    }
    return w;
  }
  clear(): void {
    this.map.clear();
  }
}

const kindOrder = (k: number): number => (k === KIND_DIR ? 0 : k === KIND_FILE ? 1 : 2);

/** Painter's order: containers before contents. */
export function sortForPaint(items: Item[]): Item[] {
  return items.sort((a, b) => kindOrder(a.kind) - kindOrder(b.kind) || a.depth - b.depth || a.slot - b.slot);
}

export function fillFor(it: Item, style: BaseStyle): RGBA {
  const t = style.theme;
  if (it.kind === KIND_DIR) return t.dirFill;
  if (style.muted?.(it)) return t.dirFill;
  // Generated and data files are drawn quieter: they are part of the repo, not its code.
  if (it.kind === KIND_FILE) return style.lensColor(it) ?? (it.flags & (FLAG_GENERATED | FLAG_DATA) ? t.generatedFill : t.fileFill);
  return style.lensColor(it) ?? t.symbolFill;
}

/**
 * One tile's batch, in tile-relative units (origin = tile corner, unit = tile size), which is
 * exactly how tiles arrive, so f32 stays precise at any zoom. Items must be in paint order.
 */
export function buildTileBatch(tile: DecodedTile, style: BaseStyle): RectBatch {
  const n = tile.items.length;
  const unit = 1 / 2 ** tile.z;
  const originX = tile.x * unit;
  const originY = tile.y * unit;
  const geom = new Float32Array(n * 4);
  const colors = new Uint8Array(n * 8);
  const stroke = new Float32Array(n);
  const inv = 1 / unit;
  const line = style.theme.line;
  for (let i = 0; i < n; i++) {
    const it = tile.items[i]!;
    geom[4 * i] = (it.x - originX) * inv;
    geom[4 * i + 1] = (it.y - originY) * inv;
    geom[4 * i + 2] = it.w * inv;
    geom[4 * i + 3] = it.h * inv;
    colors.set(fillFor(it, style), 8 * i);
    colors.set(line, 8 * i + 4);
    stroke[i] = 1;
  }
  return { originX, originY, unit, count: n, geom, colors, stroke };
}

/** Outlines drawn above all tiles (selection, "you are here", hover, pins); fills are transparent. */
export function buildOverlay(outlines: Outline[], originX: number, originY: number, unit: number): RectBatch {
  const n = outlines.length;
  const geom = new Float32Array(n * 4);
  const colors = new Uint8Array(n * 8);
  const stroke = new Float32Array(n);
  const inv = 1 / unit;
  outlines.forEach((o, i) => {
    geom.set([(o.it.x - originX) * inv, (o.it.y - originY) * inv, o.it.w * inv, o.it.h * inv], 4 * i);
    colors.set(o.color, 8 * i + 4);
    stroke[i] = o.width;
  });
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
  const v = camera.view;
  const hw = W / 2;
  const hh = H / 2;
  for (const it of items) {
    // Inline world-to-screen: this loop runs per frame over thousands of items.
    const sx = (it.x - v.cx) * v.scale + hw;
    const sy = (it.y - v.cy) * v.scale + hh;
    const sw = it.w * camera.view.scale;
    const sh = it.h * camera.view.scale;
    if (sw < 28 || sh < 14) continue;
    if (sx > W || sy > H || sx + sw < 0 || sy + sh < 0) continue;
    cands.push({ it, sx, sy, sw, sh });
  }
  cands.sort((a, b) => b.sw * b.sh - a.sw * a.sh);
  const placed = new BoxGrid(64);
  const free = (b: Box) => placed.free(b);
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
    placed.add(box);
    const color = isDir ? theme.muted : theme.foreground;
    out.push({ text, x, y, color, size: 11 });
    if (isDir && it.hidden >= MIN_AGGREGATE && sh > 60) {
      const extra = fitText(`+ ${it.hidden.toLocaleString("en-US")} small files`, avail, measure);
      const ey = Math.min(sy + sh, H) - 17;
      if (extra) {
        const eb = { x, y: ey, w: measure(extra), h: 13 };
        if (free(eb)) {
          placed.add(eb);
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
