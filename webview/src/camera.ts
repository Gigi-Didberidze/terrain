// Camera: maps world coordinates (unit square, f64) to CSS pixels.
// Motion exists only to preserve orientation: animated moves last 150 ms with ease-out.

export interface View {
  /** World point at the centre of the viewport. */
  cx: number;
  cy: number;
  /** CSS pixels per world unit. */
  scale: number;
}

export const TRANSITION_MS = 150;
const easeOut = (t: number): number => 1 - (1 - t) ** 3;

export class Camera {
  view: View = { cx: 0.5, cy: 0.3125, scale: 1000 };
  width = 1;
  height = 1;
  private anim: { from: View; to: View; start: number } | undefined;
  minScale = 100;
  maxScale = 1e9;

  resize(w: number, h: number): void {
    this.width = Math.max(1, w);
    this.height = Math.max(1, h);
  }

  toScreen(x: number, y: number): [number, number] {
    const v = this.view;
    return [(x - v.cx) * v.scale + this.width / 2, (y - v.cy) * v.scale + this.height / 2];
  }

  toWorld(sx: number, sy: number): [number, number] {
    const v = this.view;
    return [(sx - this.width / 2) / v.scale + v.cx, (sy - this.height / 2) / v.scale + v.cy];
  }

  /** Visible world rectangle. */
  bounds(): { x: number; y: number; w: number; h: number } {
    const [x0, y0] = this.toWorld(0, 0);
    return { x: x0, y: y0, w: this.width / this.view.scale, h: this.height / this.view.scale };
  }

  clamp(v: View): View {
    return { ...v, scale: Math.min(this.maxScale, Math.max(this.minScale, v.scale)) };
  }

  /** Zoom by `factor` keeping the world point under (sx, sy) fixed. Immediate (wheel input). */
  zoomAt(sx: number, sy: number, factor: number): void {
    this.anim = undefined;
    const [wx, wy] = this.toWorld(sx, sy);
    const scale = Math.min(this.maxScale, Math.max(this.minScale, this.view.scale * factor));
    this.view = {
      scale,
      cx: wx - (sx - this.width / 2) / scale,
      cy: wy - (sy - this.height / 2) / scale,
    };
  }

  panBy(dx: number, dy: number): void {
    this.anim = undefined;
    this.view = { ...this.view, cx: this.view.cx - dx / this.view.scale, cy: this.view.cy - dy / this.view.scale };
  }

  /** Pixels at the bottom covered by the legend; fitting keeps content above it. */
  bottomInset = 0;

  /** View that fits a world rect with a margin (fraction of the viewport). */
  fitView(r: { x: number; y: number; w: number; h: number }, margin = 0.06): View {
    const usableH = Math.max(1, this.height - this.bottomInset);
    const sx = (this.width * (1 - 2 * margin)) / Math.max(r.w, 1e-12);
    const sy = (usableH * (1 - 2 * margin)) / Math.max(r.h, 1e-12);
    const scale = Math.min(sx, sy);
    // Shift the centre down so the fitted rect sits in the area above the inset.
    return this.clamp({ cx: r.x + r.w / 2, cy: r.y + r.h / 2 + this.bottomInset / 2 / scale, scale });
  }

  animateTo(target: View, now = performance.now()): void {
    this.anim = { from: { ...this.view }, to: this.clamp(target), start: now };
  }

  jumpTo(target: View): void {
    this.anim = undefined;
    this.view = this.clamp(target);
  }

  /** Advances any running animation. Returns true while animating. */
  step(now: number): boolean {
    const a = this.anim;
    if (!a) return false;
    const t = Math.min(1, (now - a.start) / TRANSITION_MS);
    const k = easeOut(t);
    // Interpolate scale in log space so zooms feel uniform.
    const ls = Math.log(a.from.scale) + (Math.log(a.to.scale) - Math.log(a.from.scale)) * k;
    this.view = {
      cx: a.from.cx + (a.to.cx - a.from.cx) * k,
      cy: a.from.cy + (a.to.cy - a.from.cy) * k,
      scale: Math.exp(ls),
    };
    if (t >= 1) {
      this.view = a.to;
      this.anim = undefined;
    }
    return true;
  }

  get animating(): boolean {
    return this.anim !== undefined;
  }
}
