// Hierarchical edge bundling (Holten 2006). An edge's control points are the centres of the
// containers between its endpoints (up to their common ancestor and down again); a cubic
// B-spline through them, straightened by beta, makes edges that share structure share a route.

export interface RectLike {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Bundling strength: 1 follows the hierarchy fully, 0 is a straight line. */
export const BETA = 0.85;
const SAMPLES_PER_SPAN = 8;

/** Points (world coordinates, flat [x0, y0, x1, y1, ...]) of the bundled curve along `route`. */
export function bundledCurve(route: readonly RectLike[], beta = BETA): number[] {
  if (route.length === 0) return [];
  const pts = route.map((r) => [r.x + r.w / 2, r.y + r.h / 2] as [number, number]);
  if (pts.length === 1) return [...pts[0]!];
  const n = pts.length;
  const [x0, y0] = pts[0]!;
  const [xn, yn] = pts[n - 1]!;
  // Straighten towards the chord so tightly packed routes still fan out a little.
  const ctrl = pts.map(([x, y], i) => {
    const t = i / (n - 1);
    return [beta * x + (1 - beta) * (x0 + t * (xn - x0)), beta * y + (1 - beta) * (y0 + t * (yn - y0))] as [number, number];
  });
  if (n === 2) return [x0, y0, xn, yn];
  // Clamped uniform cubic B-spline: repeat the ends so the curve starts and ends on them.
  const c = [ctrl[0]!, ctrl[0]!, ...ctrl, ctrl[n - 1]!, ctrl[n - 1]!];
  const out: number[] = [];
  for (let i = 0; i + 3 < c.length; i++) {
    const [p0, p1, p2, p3] = [c[i]!, c[i + 1]!, c[i + 2]!, c[i + 3]!];
    for (let s = 0; s < SAMPLES_PER_SPAN; s++) {
      const t = s / SAMPLES_PER_SPAN;
      const t2 = t * t;
      const t3 = t2 * t;
      const b0 = (1 - 3 * t + 3 * t2 - t3) / 6;
      const b1 = (4 - 6 * t2 + 3 * t3) / 6;
      const b2 = (1 + 3 * t + 3 * t2 - 3 * t3) / 6;
      const b3 = t3 / 6;
      out.push(b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0], b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1]);
    }
  }
  out.push(xn, yn);
  return out;
}
