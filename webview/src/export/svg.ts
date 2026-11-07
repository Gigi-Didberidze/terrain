// "Export View as SVG": the frame the renderer would draw, as vector shapes in CSS pixels.
// Rectangles are clipped to their tile and to the viewport, like on screen; colours are the
// theme's, so the export matches what the user sees.
import type { Frame, RectBatch } from "../render/types.ts";

const rgba = (c: ArrayLike<number>, i = 0) =>
  c[i + 3] === 255 ? `rgb(${c[i]},${c[i + 1]},${c[i + 2]})` : `rgba(${c[i]},${c[i + 1]},${c[i + 2]},${+(c[i + 3]! / 255).toFixed(3)})`;

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const n = (v: number) => +v.toFixed(2);

export function frameToSvg(frame: Frame, width: number, height: number, fontFamily: string): string {
  const v = frame.view;
  const sx = (x: number) => (x - v.cx) * v.scale + width / 2;
  const sy = (y: number) => (y - v.cy) * v.scale + height / 2;
  const out: string[] = [];
  out.push(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="${esc(fontFamily)}">`,
    `<rect width="${width}" height="${height}" fill="${rgba(frame.background)}"/>`,
  );
  const rects = (b: RectBatch, clip?: { x: number; y: number; w: number; h: number }) => {
    const cx0 = clip ? Math.max(0, sx(clip.x)) : 0;
    const cy0 = clip ? Math.max(0, sy(clip.y)) : 0;
    const cx1 = clip ? Math.min(width, sx(clip.x + clip.w)) : width;
    const cy1 = clip ? Math.min(height, sy(clip.y + clip.h)) : height;
    for (let i = 0; i < b.count; i++) {
      const x0 = sx(b.originX + b.geom[4 * i]! * b.unit);
      const y0 = sy(b.originY + b.geom[4 * i + 1]! * b.unit);
      const x1 = x0 + b.geom[4 * i + 2]! * b.unit * v.scale;
      const y1 = y0 + b.geom[4 * i + 3]! * b.unit * v.scale;
      const ax = Math.max(x0, cx0);
      const ay = Math.max(y0, cy0);
      const bx = Math.min(x1, cx1);
      const by = Math.min(y1, cy1);
      if (bx - ax < 0.25 || by - ay < 0.25) continue;
      const fillA = b.colors[8 * i + 3]!;
      const strokeA = b.colors[8 * i + 7]!;
      const w = b.stroke[i]!;
      const fill = fillA ? `fill="${rgba(b.colors, 8 * i)}"` : `fill="none"`;
      const stroke = strokeA && w > 0 ? ` stroke="${rgba(b.colors, 8 * i + 4)}" stroke-width="${n(w)}"` : "";
      out.push(`<rect x="${n(ax)}" y="${n(ay)}" width="${n(bx - ax)}" height="${n(by - ay)}" ${fill}${stroke}/>`);
    }
  };
  for (const t of frame.tiles) rects(t.batch, t.clip);
  if (frame.overlay) rects(frame.overlay);
  for (const l of frame.lines) {
    if (l.points.length < 4) continue;
    const pts = [];
    for (let i = 0; i < l.points.length; i += 2) pts.push(`${n(l.points[i]!)},${n(l.points[i + 1]!)}`);
    const dash = l.style === 1 ? ` stroke-dasharray="5 3"` : l.style === 2 ? ` stroke-dasharray="1 3"` : "";
    out.push(`<polyline points="${pts.join(" ")}" fill="none" stroke="${rgba(l.color)}" stroke-width="${n(l.width)}"${dash}/>`);
  }
  for (const t of frame.labels) {
    // Renderer labels are positioned by their top-left; SVG text by its baseline.
    out.push(`<text x="${n(t.x)}" y="${n(t.y + t.size * 0.82)}" font-size="${t.size}" fill="${rgba(t.color)}">${esc(t.text)}</text>`);
  }
  out.push("</svg>");
  return out.join("\n");
}
