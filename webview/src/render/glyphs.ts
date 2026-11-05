// Glyph atlas. Labels are always drawn at 11, 12 or 13 CSS px, so glyphs are rasterised at
// exactly that size times devicePixelRatio: crisper than a scaled SDF at these sizes.
// Glyphs are added on demand, so any Unicode label renders.

export interface Glyph {
  /** Atlas rect in device px. */
  u: number;
  v: number;
  w: number;
  h: number;
  /** Advance in CSS px. */
  advance: number;
}

const ATLAS = 2048;
const PAD = 1;

export class GlyphAtlas {
  readonly canvas: HTMLCanvasElement | OffscreenCanvas;
  private readonly ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
  private readonly glyphs = new Map<string, Glyph>();
  private penX = PAD;
  private penY = PAD;
  private rowH = 0;
  private dpr = 1;
  family = "monospace";
  /** Bumped whenever new glyphs are drawn, so the GPU texture can be re-uploaded. */
  version = 0;
  readonly size = ATLAS;

  constructor() {
    this.canvas = typeof OffscreenCanvas !== "undefined" ? new OffscreenCanvas(ATLAS, ATLAS) : Object.assign(document.createElement("canvas"), { width: ATLAS, height: ATLAS });
    const ctx = this.canvas.getContext("2d", { willReadFrequently: false }) as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
    if (!ctx) throw new Error("2d context unavailable");
    this.ctx = ctx;
  }

  configure(family: string, dpr: number): void {
    if (family === this.family && dpr === this.dpr) return;
    this.family = family;
    this.dpr = dpr;
    this.reset();
  }

  private reset(): void {
    this.glyphs.clear();
    this.ctx.clearRect(0, 0, ATLAS, ATLAS);
    this.penX = PAD;
    this.penY = PAD;
    this.rowH = 0;
    this.version++;
  }

  private font(size: number): string {
    return `${size * this.dpr}px ${this.family}`;
  }

  glyph(ch: string, size: number): Glyph {
    const key = `${size}\u0000${ch}`;
    const g = this.glyphs.get(key);
    if (g) return g;
    const ctx = this.ctx;
    ctx.font = this.font(size);
    const m = ctx.measureText(ch);
    const w = Math.ceil(m.width) + 2;
    const h = Math.ceil(size * this.dpr * 1.35);
    if (this.penX + w + PAD > ATLAS) {
      this.penX = PAD;
      this.penY += this.rowH + PAD;
      this.rowH = 0;
    }
    if (this.penY + h + PAD > ATLAS) {
      // Atlas full: start over. Rare (thousands of distinct glyphs).
      this.reset();
      return this.glyph(ch, size);
    }
    ctx.fillStyle = "#fff";
    ctx.textBaseline = "top";
    ctx.fillText(ch, this.penX + 1, this.penY + Math.round(size * this.dpr * 0.12));
    const out: Glyph = { u: this.penX, v: this.penY, w, h, advance: m.width / this.dpr };
    this.glyphs.set(key, out);
    this.penX += w + PAD;
    this.rowH = Math.max(this.rowH, h);
    this.version++;
    return out;
  }

  measure(text: string, size: number): number {
    let w = 0;
    for (const ch of text) w += this.glyph(ch, size).advance;
    return w;
  }
}
