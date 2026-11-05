// Canvas2D fallback with the same scene model as the WebGL2 renderer. Used when WebGL2 is not
// available (some remote / software-rendered environments).
import type { View } from "../camera.ts";
import type { Frame, RectBatch, Renderer } from "./types.ts";

const css = (c: ArrayLike<number>, o = 0): string => `rgba(${c[o]},${c[o + 1]},${c[o + 2]},${(c[o + 3]! / 255).toFixed(3)})`;

export class Canvas2DRenderer implements Renderer {
  readonly kind = "canvas2d" as const;
  private readonly ctx: CanvasRenderingContext2D;
  private readonly canvas: HTMLCanvasElement;
  private dpr = 1;
  private w = 1;
  private h = 1;
  private family = "monospace";

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) throw new Error("2d context unavailable");
    this.ctx = ctx;
  }

  setFont(family: string): void {
    this.family = family;
  }

  resize(cssW: number, cssH: number, dpr: number): void {
    this.dpr = dpr;
    this.w = cssW;
    this.h = cssH;
    this.canvas.width = Math.round(cssW * dpr);
    this.canvas.height = Math.round(cssH * dpr);
    this.canvas.style.width = `${cssW}px`;
    this.canvas.style.height = `${cssH}px`;
  }

  measure(text: string, size: 11 | 12 | 13): number {
    this.ctx.font = `${size}px ${this.family}`;
    return this.ctx.measureText(text).width;
  }

  private drawBatch(b: RectBatch, view: View): void {
    const ctx = this.ctx;
    const s = view.scale * b.unit;
    const ox = (b.originX - view.cx) * view.scale + this.w / 2;
    const oy = (b.originY - view.cy) * view.scale + this.h / 2;
    for (let i = 0; i < b.count; i++) {
      const x0 = Math.round(b.geom[4 * i]! * s + ox);
      const y0 = Math.round(b.geom[4 * i + 1]! * s + oy);
      const x1 = Math.max(x0 + 1, Math.round((b.geom[4 * i]! + b.geom[4 * i + 2]!) * s + ox));
      const y1 = Math.max(y0 + 1, Math.round((b.geom[4 * i + 1]! + b.geom[4 * i + 3]!) * s + oy));
      if (x1 < 0 || y1 < 0 || x0 > this.w || y0 > this.h) continue;
      const sw = b.stroke[i]!;
      if (sw > 0 && b.colors[8 * i + 7]! > 0) {
        if (b.colors[8 * i + 3]! > 0) {
          ctx.fillStyle = css(b.colors, 8 * i + 4);
          ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
          if (x1 - x0 > 2 * sw && y1 - y0 > 2 * sw) {
            ctx.fillStyle = css(b.colors, 8 * i);
            ctx.fillRect(x0 + sw, y0 + sw, x1 - x0 - 2 * sw, y1 - y0 - 2 * sw);
          }
        } else {
          ctx.strokeStyle = css(b.colors, 8 * i + 4);
          ctx.lineWidth = sw;
          ctx.strokeRect(x0 + sw / 2, y0 + sw / 2, x1 - x0 - sw, y1 - y0 - sw);
        }
      } else if (b.colors[8 * i + 3]! > 0) {
        ctx.fillStyle = css(b.colors, 8 * i);
        ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
      }
    }
  }

  draw(f: Frame): void {
    const ctx = this.ctx;
    const { view, labels, lines } = f;
    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.fillStyle = css(f.background);
    ctx.fillRect(0, 0, this.w, this.h);
    for (const t of f.tiles) {
      const x0 = Math.round((t.clip.x - view.cx) * view.scale + this.w / 2);
      const y0 = Math.round((t.clip.y - view.cy) * view.scale + this.h / 2);
      const x1 = Math.round((t.clip.x + t.clip.w - view.cx) * view.scale + this.w / 2);
      const y1 = Math.round((t.clip.y + t.clip.h - view.cy) * view.scale + this.h / 2);
      if (x1 <= 0 || y1 <= 0 || x0 >= this.w || y0 >= this.h) continue;
      ctx.save();
      ctx.beginPath();
      ctx.rect(x0, y0, x1 - x0, y1 - y0);
      ctx.clip();
      this.drawBatch(t.batch, view);
      ctx.restore();
    }
    if (f.overlay) this.drawBatch(f.overlay, view);
    for (const l of lines) {
      ctx.strokeStyle = css(l.color);
      ctx.lineWidth = l.width;
      ctx.setLineDash(l.style === 1 ? [5, 3] : l.style === 2 ? [1.5, 2.5] : []);
      ctx.beginPath();
      for (let i = 0; i < l.points.length; i += 2) {
        if (i === 0) ctx.moveTo(l.points[0]!, l.points[1]!);
        else ctx.lineTo(l.points[i]!, l.points[i + 1]!);
      }
      ctx.stroke();
    }
    ctx.setLineDash([]);
    ctx.textBaseline = "top";
    for (const l of labels) {
      ctx.font = `${l.size}px ${this.family}`;
      ctx.fillStyle = css(l.color);
      ctx.fillText(l.text, Math.round(l.x), Math.round(l.y + l.size * 0.12));
    }
  }

  dispose(): void {}
}
