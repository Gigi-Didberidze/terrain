// Scene data shared by the WebGL2 renderer and the Canvas2D fallback.
import type { View } from "../camera.ts";

/** Rectangles in "local" units: world = origin + local * unit. Kept small so f32 stays precise at deep zoom. */
export interface RectBatch {
  originX: number;
  originY: number;
  unit: number;
  count: number;
  /** x, y, w, h per instance (local units). */
  geom: Float32Array;
  /** fill RGBA then stroke RGBA per instance. */
  colors: Uint8Array;
  /** Stroke width in CSS px per instance. */
  stroke: Float32Array;
}

export interface Label {
  text: string;
  /** Top-left in CSS px. */
  x: number;
  y: number;
  color: [number, number, number, number];
  /** 11, 12 or 13 px only. */
  size: 11 | 12 | 13;
}

/** Overlay lines (edges, pins) in CSS px. */
export interface Line {
  points: number[];
  color: [number, number, number, number];
  width: number;
  /** 0 = solid, 1 = dashed, 2 = dotted. */
  style: 0 | 1 | 2;
}

export interface Renderer {
  readonly kind: "webgl2" | "canvas2d";
  resize(cssWidth: number, cssHeight: number, dpr: number): void;
  setRects(batch: RectBatch): void;
  draw(view: View, background: [number, number, number, number], labels: Label[], lines: Line[]): void;
  measure(text: string, size: 11 | 12 | 13): number;
  setFont(family: string): void;
  dispose(): void;
}

export function emptyBatch(): RectBatch {
  return { originX: 0, originY: 0, unit: 1, count: 0, geom: new Float32Array(0), colors: new Uint8Array(0), stroke: new Float32Array(0) };
}
