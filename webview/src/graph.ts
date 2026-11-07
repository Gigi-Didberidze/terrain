// Neighbourhood and Path views: a small node-link diagram drawn with the map's renderer.
// Reads left to right: everything on the left uses what is to its right. Selection is shared
// with the map; edge styles follow confidence (solid confirmed, dashed name-matched, dotted
// history), and colours follow the map's edges (purple uses, orange used by).
import type { NeighbourhoodResult, NodeRef, PathResult } from "../../protocol/generated/protocol.ts";
import { Camera } from "./camera.ts";
import { BOX_H, layered, type Layered } from "./layout/layered.ts";
import { mix, type RGBA, type Theme } from "./render/theme.ts";
import type { Frame, Label, Line, RectBatch } from "./render/types.ts";
import { fitText } from "./scene.ts";

export type GraphKind = "neighbourhood" | "path";

export interface GraphNode {
  slot: number;
  ref: NodeRef;
  column: number;
  load: number;
  /** Second label line in the hover card, e.g. "used by 12 · uses 3". */
  detail: string;
}

export interface GraphEdge {
  from: number;
  to: number;
  /** 0 solid (confirmed), 1 dashed (name-matched), 2 dotted (history). */
  style: 0 | 1 | 2;
  /** True for edges into the centre's side (left of it): drawn as "used by". */
  usedBy: boolean;
}

export interface GraphModel {
  kind: GraphKind;
  title: string;
  /** The centre (Neighbourhood) or the start (Path). */
  focus: number;
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** Column captions, by column. */
  captions: Map<number, string>;
  /** One line for the legend. */
  summary: string;
}

const MIN_W = 64;
const MAX_W = 220;
/** Label size in the diagram; boxes are sized for it at zoom 1. */
const LABEL = 11;
const PAD = 8;

const styleOf = (c: string): 0 | 1 | 2 => (c === "semantic" ? 0 : c === "historical" ? 2 : 1);
const count = (shown: number, total: number) => (total > shown ? `${shown} of ${total.toLocaleString("en-US")}` : `${total.toLocaleString("en-US")}`);

export function neighbourhoodModel(r: NeighbourhoodResult): GraphModel {
  const nodes = r.nodes.map((n) => ({
    slot: n.node.slot,
    ref: n.node,
    column: n.column,
    load: n.load,
    detail: `used by ${n.inCount.toLocaleString("en-US")} · uses ${n.outCount.toLocaleString("en-US")}`,
  }));
  const col = new Map(nodes.map((n) => [n.slot, n.column]));
  const edges = r.edges.map((e) => ({ from: e.from, to: e.to, style: styleOf(e.confidence), usedBy: (col.get(e.to) ?? 2) <= 2 }));
  const per = (c: number) => nodes.filter((n) => n.column === c).length;
  const centre = nodes.find((n) => n.slot === r.centre);
  const captions = new Map<number, string>([
    [0, per(0) ? "… and what uses those" : ""],
    [1, per(1) || r.dependents ? `used by · ${count(per(1), r.dependents)}` : "used by nothing"],
    [2, ""],
    [3, per(3) || r.dependencies ? `uses · ${count(per(3), r.dependencies)}` : "uses nothing"],
    [4, per(4) ? "… and what those use" : ""],
  ]);
  return {
    kind: "neighbourhood",
    title: centre ? centre.ref.name : "",
    focus: r.centre,
    nodes,
    edges,
    captions,
    summary: `Neighbourhood of ${centre?.ref.name ?? "?"}: left uses right · ${r.dependents.toLocaleString("en-US")} use it · it uses ${r.dependencies.toLocaleString("en-US")}`,
  };
}

export function pathModel(r: PathResult): GraphModel {
  const seen = new Map<number, GraphNode>();
  const edges: GraphEdge[] = [];
  const edgeKeys = new Set<string>();
  const last = Math.max(1, ...r.paths.map((p) => p.nodes.length - 1));
  for (const p of r.paths) {
    p.nodes.forEach((n, i) => {
      // The two ends share a column across paths; the rest keep their step along the path.
      const column = i === p.nodes.length - 1 ? last : i;
      const prev = seen.get(n.slot);
      if (!prev) seen.set(n.slot, { slot: n.slot, ref: n, column, load: 0, detail: "" });
      else prev.column = Math.max(prev.column, column);
      if (i > 0) {
        const from = p.nodes[i - 1]!.slot;
        const key = `${from}>${n.slot}`;
        if (!edgeKeys.has(key)) {
          edgeKeys.add(key);
          edges.push({ from, to: n.slot, style: 1, usedBy: false });
        }
      }
    });
  }
  const [a, b] = r.reversed ? [r.to, r.from] : [r.from, r.to];
  const summary = !r.paths.length
    ? `No dependency chain between ${r.from.name} and ${r.to.name}, in either direction`
    : `${r.paths.length} shortest chain${r.paths.length > 1 ? "s" : ""}: ${a.name} uses ${b.name} in ${r.paths[0]!.nodes.length - 1} step${r.paths[0]!.nodes.length > 2 ? "s" : ""}${r.reversed ? " (the other way round from what was asked)" : ""}`;
  return {
    kind: "path",
    title: `${a.name} → ${b.name}`,
    focus: a.slot,
    nodes: [...seen.values()],
    edges,
    captions: new Map(),
    summary,
  };
}

export class GraphView {
  readonly camera = new Camera();
  model: GraphModel | null = null;
  private layout: Layered | null = null;
  private labelsFor = new Map<number, string>();
  private measure: (t: string) => number = (t) => t.length * 7;

  constructor() {
    this.camera.view = { cx: 0, cy: 0, scale: 1 };
    this.camera.minScale = 0.15;
    this.camera.maxScale = 4;
  }

  set(model: GraphModel, measure: (t: string) => number): void {
    this.model = model;
    this.measure = measure;
    this.labelsFor.clear();
    const nodes = model.nodes.map((n, i) => {
      const w = Math.min(MAX_W, Math.max(MIN_W, measure(n.ref.name) + 2 * PAD));
      return { id: n.slot, column: n.column, width: w, rank: i };
    });
    this.layout = layered(nodes, model.edges);
    for (const n of model.nodes) {
      const p = this.layout.nodes.get(n.slot)!;
      this.labelsFor.set(n.slot, fitText(n.ref.name, p.w - 2 * PAD, measure) ?? "");
    }
  }

  /**
   * Fits the diagram, but never below zoom 1, where labels fit their boxes: a diagram
   * wider than the panel is centred on its focus and panned instead of shrunk.
   */
  fit(animate: boolean): void {
    if (!this.layout || !this.model) return;
    const b = this.layout.bounds;
    // Leave room for captions above and the legend below.
    const r = { x: b.x - 24, y: b.y - 40, w: b.w + 48, h: b.h + 72 };
    const v = this.camera.fitView(r, 0.02);
    v.scale = Math.min(1.25, v.scale);
    if (v.scale < 1) {
      v.scale = 1;
      const f = this.layout.nodes.get(this.model.focus);
      if (f && b.w * v.scale > this.camera.width) v.cx = f.x + f.w / 2;
    }
    if (animate) this.camera.animateTo(v);
    else this.camera.jumpTo(v);
  }

  has(slot: number): boolean {
    return !!this.layout?.nodes.has(slot);
  }

  hit(sx: number, sy: number): number | null {
    if (!this.layout) return null;
    const [x, y] = this.camera.toWorld(sx, sy);
    for (const p of this.layout.nodes.values()) if (x >= p.x && x <= p.x + p.w && y >= p.y && y <= p.y + p.h) return p.id;
    return null;
  }

  node(slot: number): GraphNode | undefined {
    return this.model?.nodes.find((n) => n.slot === slot);
  }

  /** Arrow keys: up/down within a column, left/right along edges (nearest by height). */
  move(from: number, dx: number, dy: number): number | null {
    const l = this.layout;
    const m = this.model;
    if (!l || !m) return null;
    const p = l.nodes.get(from);
    if (!p) return m.focus;
    if (dy) {
      const col = l.columns.find((c) => c.includes(from))!;
      return col[col.indexOf(from) + dy] ?? null;
    }
    const linked = m.edges
      .filter((e) => (dx > 0 ? e.from === from : e.to === from))
      .map((e) => (dx > 0 ? e.to : e.from));
    const candidates = linked.length ? linked : [...l.nodes.values()].filter((q) => q.column === p.column + dx).map((q) => q.id);
    let best: number | null = null;
    let bestD = Infinity;
    for (const c of candidates) {
      const q = l.nodes.get(c);
      if (!q) continue;
      const d = Math.abs(q.y - p.y) + Math.abs(q.column - p.column) * 1000;
      if (d < bestD) {
        bestD = d;
        best = c;
      }
    }
    return best;
  }

  /** Frame for the renderer. `selected` gets the accent outline; `hover` a quiet one. */
  frame(theme: Theme, selected: number | null, hover: number | null, highlight: ReadonlySet<number>): Frame {
    const l = this.layout;
    const m = this.model;
    const cam = this.camera;
    const empty: Frame = { view: cam.view, background: theme.background, tiles: [], overlay: null, labels: [], lines: [] };
    if (!l || !m) return empty;
    const nodes = [...l.nodes.values()];
    const geom = new Float32Array(nodes.length * 4);
    const colors = new Uint8Array(nodes.length * 8);
    const stroke = new Float32Array(nodes.length);
    nodes.forEach((p, i) => {
      geom.set([p.x, p.y, p.w, p.h], 4 * i);
      const n = this.node(p.id)!;
      const isFocus = p.id === m.focus;
      const fill = isFocus ? mix(theme.symbolFill, theme.foreground, 0.08) : n.ref.kind === "file" ? theme.fileFill : theme.symbolFill;
      const line: RGBA = p.id === selected ? theme.accent : highlight.has(p.id) ? theme.foreground : p.id === hover ? mix(theme.line, theme.foreground, 0.5) : theme.line;
      colors.set(fill, 8 * i);
      colors.set(line, 8 * i + 4);
      stroke[i] = p.id === selected ? 2 : 1;
    });
    const overlay: RectBatch = { originX: 0, originY: 0, unit: 1, count: nodes.length, geom, colors, stroke };

    const labels: Label[] = [];
    const scale = cam.view.scale;
    // Labels keep their size; below zoom 1 they are re-fitted to the smaller boxes, and
    // dropped once boxes are too short to hold a line of text.
    if (BOX_H * scale >= 12) {
      for (const p of nodes) {
        let text = this.labelsFor.get(p.id);
        if (!text) continue;
        if (scale < 1) text = fitText(this.node(p.id)!.ref.name, p.w * scale - 2 * PAD * scale, this.measure) ?? "";
        if (!text) continue;
        const [sx, sy] = cam.toScreen(p.x + PAD, p.y);
        labels.push({ text, x: sx, y: sy + (BOX_H * scale - LABEL) / 2, color: theme.foreground, size: LABEL });
      }
    }
    for (const [c, text] of m.captions) {
      const col = l.columnX.get(c);
      if (!text || !col) continue;
      const [sx, sy] = cam.toScreen(col.x, l.bounds.y - 26);
      labels.push({ text, x: sx, y: sy, color: theme.muted, size: 11 });
    }

    const lines: Line[] = [];
    const uses = [...theme.categorical[3]!.slice(0, 3), 200] as RGBA;
    const usedBy = [...theme.categorical[1]!.slice(0, 3), 200] as RGBA;
    for (const e of m.edges) {
      const a = l.nodes.get(e.from);
      const b = l.nodes.get(e.to);
      if (!a || !b) continue;
      const focused = selected !== null && (e.from === selected || e.to === selected);
      const color = e.usedBy ? usedBy : uses;
      lines.push({
        points: curve(cam, a.x + a.w, a.y + a.h / 2, b.x, b.y + b.h / 2),
        color: focused ? color : ([color[0], color[1], color[2], 110] as RGBA),
        width: focused ? 1.5 : 1,
        style: e.style,
      });
    }
    return { view: cam.view, background: theme.background, tiles: [], overlay, labels, lines };
  }
}

/** A horizontal S-curve between two box sides, in CSS px. */
function curve(cam: Camera, x0: number, y0: number, x1: number, y1: number): number[] {
  const pts: number[] = [];
  const dx = Math.max(24, (x1 - x0) / 2);
  for (let i = 0; i <= 16; i++) {
    const t = i / 16;
    const u = 1 - t;
    const x = u * u * u * x0 + 3 * u * u * t * (x0 + dx) + 3 * u * t * t * (x1 - dx) + t * t * t * x1;
    const y = u * u * u * y0 + 3 * u * u * t * y0 + 3 * u * t * t * y1 + t * t * t * y1;
    const [sx, sy] = cam.toScreen(x, y);
    pts.push(sx, sy);
  }
  return pts;
}
