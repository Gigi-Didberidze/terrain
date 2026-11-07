// Layered layout for the Neighbourhood and Path views. Columns are given (distance from the
// centre, or position along a path), so what remains of the Sugiyama method is ordering each
// column to reduce crossings (barycentre sweeps) and assigning coordinates. A few dozen nodes
// lay out in well under a millisecond (see DECISIONS.md D39).

export interface LayerNode {
  id: number;
  column: number;
  /** Box width in world units (CSS px at zoom 1). */
  width: number;
  /** Initial order within the column (lower first): the strongest edges come first. */
  rank: number;
}

export interface LayerEdge {
  from: number;
  to: number;
}

export interface Placed {
  id: number;
  column: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Layered {
  nodes: Map<number, Placed>;
  /** Column order, top to bottom. */
  columns: number[][];
  /** Left edge and width of each column, by column number. */
  columnX: Map<number, { x: number; w: number }>;
  bounds: { x: number; y: number; w: number; h: number };
}

export const BOX_H = 22;
export const ROW_GAP = 8;
export const COLUMN_GAP = 96;

export function layered(nodes: LayerNode[], edges: LayerEdge[], sweeps = 4): Layered {
  const cols = [...new Set(nodes.map((n) => n.column))].sort((a, b) => a - b);
  const byCol = new Map<number, LayerNode[]>();
  for (const c of cols) byCol.set(c, []);
  for (const n of nodes) byCol.get(n.column)!.push(n);
  for (const list of byCol.values()) list.sort((a, b) => a.rank - b.rank || a.id - b.id);

  const neighbours = new Map<number, number[]>();
  const link = (a: number, b: number) => {
    if (!neighbours.has(a)) neighbours.set(a, []);
    neighbours.get(a)!.push(b);
  };
  for (const e of edges) {
    link(e.from, e.to);
    link(e.to, e.from);
  }
  const column = new Map(nodes.map((n) => [n.id, n.column]));
  const pos = new Map<number, number>();
  const setPositions = () => {
    for (const list of byCol.values()) list.forEach((n, i) => pos.set(n.id, i - (list.length - 1) / 2));
  };
  setPositions();
  // Barycentre sweeps: order each column by the mean position of its neighbours in the column
  // just swept (left to right, then right to left). Ties keep the previous order, so the
  // strongest-first ranking survives where the edges do not say otherwise.
  const sweep = (order: number[]) => {
    for (let k = 1; k < order.length; k++) {
      const c = order[k]!;
      const prev = order[k - 1]!;
      const list = byCol.get(c)!;
      const bary = new Map<number, number>();
      for (const n of list) {
        const ps = (neighbours.get(n.id) ?? []).filter((m) => column.get(m) === prev).map((m) => pos.get(m)!);
        bary.set(n.id, ps.length ? ps.reduce((s, v) => s + v, 0) / ps.length : pos.get(n.id)!);
      }
      list.sort((a, b) => bary.get(a.id)! - bary.get(b.id)! || pos.get(a.id)! - pos.get(b.id)!);
      list.forEach((n, i) => pos.set(n.id, i - (list.length - 1) / 2));
    }
  };
  // Sweep outwards from the column with the fewest nodes (the centre, for a neighbourhood).
  const pivot = cols.reduce((best, c) => (byCol.get(c)!.length < byCol.get(best)!.length ? c : best), cols[0] ?? 0);
  const right = cols.filter((c) => c >= pivot);
  const left = cols.filter((c) => c <= pivot).reverse();
  for (let i = 0; i < sweeps; i++) {
    sweep(right);
    sweep(left);
  }

  const columnX = new Map<number, { x: number; w: number }>();
  let x = 0;
  for (const c of cols) {
    const w = Math.max(...byCol.get(c)!.map((n) => n.width));
    columnX.set(c, { x, w });
    x += w + COLUMN_GAP;
  }
  const placed = new Map<number, Placed>();
  let minY = 0;
  let maxY = 0;
  for (const c of cols) {
    const { x: cx, w: cw } = columnX.get(c)!;
    for (const n of byCol.get(c)!) {
      const y = pos.get(n.id)! * (BOX_H + ROW_GAP) - BOX_H / 2;
      // Right-align boxes left of the centre column and left-align the rest, so edges stay short.
      const nx = c < pivot ? cx + cw - n.width : c > pivot ? cx : cx + (cw - n.width) / 2;
      placed.set(n.id, { id: n.id, column: c, x: nx, y, w: n.width, h: BOX_H });
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y + BOX_H);
    }
  }
  return {
    nodes: placed,
    columns: cols.map((c) => byCol.get(c)!.map((n) => n.id)),
    columnX,
    bounds: { x: 0, y: minY, w: Math.max(0, x - COLUMN_GAP), h: maxY - minY },
  };
}

/** Number of edge crossings between adjacent columns (for tests and diagnostics). */
export function crossings(l: Layered, edges: LayerEdge[]): number {
  const segs = edges
    .map((e) => [l.nodes.get(e.from), l.nodes.get(e.to)] as const)
    .filter(([a, b]) => a && b && Math.abs(a.column - b.column) === 1)
    .map(([a, b]) => (a!.column < b!.column ? [a!, b!] : [b!, a!]));
  let n = 0;
  for (let i = 0; i < segs.length; i++)
    for (let j = i + 1; j < segs.length; j++) {
      const [a1, b1] = segs[i]!;
      const [a2, b2] = segs[j]!;
      if (a1.column !== a2.column) continue;
      if ((a1.y - a2.y) * (b1.y - b2.y) < 0) n++;
    }
  return n;
}
