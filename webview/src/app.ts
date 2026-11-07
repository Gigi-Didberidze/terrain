// The map view: tile streaming, scene assembly, input and the render loop.
// Frames are drawn only when something changes; nothing animates on its own.
import type { EdgesResult, MapInfo, NodeInfo } from "../../protocol/generated/protocol.ts";
import type { HostToView, ViewTile } from "../../protocol/webview.ts";
import { Camera } from "./camera.ts";
import { Host } from "./host.ts";
import { bundledCurve } from "./layout/bundle.ts";
import { Canvas2DRenderer } from "./render/canvas2d.ts";
import { GLRenderer } from "./render/gl.ts";
import { readTheme, type Theme } from "./render/theme.ts";
import type { Line, Renderer, TileDraw } from "./render/types.ts";
import { buildLabels, buildOverlay, buildTileBatch, hereMarker, sortForPaint, WidthCache, type BaseStyle, type Outline } from "./scene.ts";
import { decodeTile, KIND_DIR, KIND_FILE, TileCache, tileKey, type DecodedTile, type Item } from "./tiles.ts";
import { Help } from "./ui/help.ts";
import { LENSES, lensColor, type LensId } from "./lenses.ts";
import { HoverCard } from "./ui/hover.ts";
import { Legend } from "./ui/legend.ts";
import { SearchBox } from "./ui/search.ts";

/** Tiles per request: small groups arrive (and get drawn) progressively. */
const MAX_TILE_REQUEST = 6;
/** Main-thread time per frame spent decoding arrived tiles. */
const DECODE_BUDGET_MS = 3;
/** Frame time after which no new tile batches are built (the rest wait for the next frame). */
const BUILD_BUDGET_MS = 7;

export class App {
  private readonly host = new Host();
  private readonly root: HTMLElement;
  private readonly canvas: HTMLCanvasElement;
  private renderer!: Renderer;
  private theme!: Theme;
  private readonly camera = new Camera();
  private info: MapInfo | undefined;
  private cache = new TileCache();
  private stale = new TileCache();
  /** Tiles drawn in the last frame, each with the clip rect of the key it stands in for. */
  private drawn: { tile: DecodedTile; clip: { x: number; y: number; w: number; h: number } }[] = [];
  /** Bumped when fills change (theme, lens); tile batches rebuild lazily. */
  private styleVersion = 0;
  private frame = 0;
  private lens: LensId = 1;
  /** Shown in the legend for a moment when a lens is not available yet. */
  private lensNotice = "";
  /** Dependency edges per node, fetched on demand for the selection, hover and pins. */
  private readonly edgeSets = new Map<number, EdgesResult>();
  private readonly edgePending = new Set<number>();
  private widths!: WidthCache;
  /** Tiles that arrived but are not decoded yet (decoded a few per frame). */
  private readonly inbox: { cache: TileCache; tile: ViewTile }[] = [];

  private selected: number | null = null;
  private here: number | null = null;
  private hover: number | null = null;
  private readonly pinned = new Set<number>();
  private readonly history: number[] = [];
  private follow = true;
  private progress = "";

  private readonly legend: Legend;
  private readonly hoverCard: HoverCard;
  private readonly search: SearchBox;
  private readonly help: Help;
  private readonly stateEl: HTMLElement;
  private hoverTimer = 0;
  private hoverSeq = 0;
  private pendingFollow: NodeInfo | null = null;

  constructor(root: HTMLElement) {
    this.root = root;
    this.canvas = document.createElement("canvas");
    this.canvas.className = "map";
    this.canvas.tabIndex = 0;
    this.canvas.setAttribute("aria-label", "Code map. Press question mark for keyboard shortcuts.");
    root.append(this.canvas);
    this.stateEl = document.createElement("div");
    this.stateEl.className = "state";
    this.stateEl.setAttribute("role", "status");
    root.append(this.stateEl);
    this.legend = new Legend(root);
    this.legend.onCountClick = () => this.search.show();
    this.hoverCard = new HoverCard(root);
    this.search = new SearchBox(root, (q) => this.host.request("search", { query: q, limit: 30 }));
    this.search.onPick = (h) => void this.reveal(h.slot, true);
    this.search.onClose = () => this.canvas.focus();
    this.help = new Help(root);
    this.camera.bottomInset = 24;
    this.follow = this.host.state.follow !== false;
    const savedLens = Number(this.host.state.lens);
    if (savedLens === 1 || savedLens === 2) this.lens = savedLens;

    this.initRenderer();
    this.bindInput();
    new ResizeObserver(() => this.resize()).observe(root);
    new MutationObserver(() => this.onThemeChange()).observe(document.body, { attributes: true, attributeFilter: ["class"] });
    this.host.onMessage = (m) => this.onHost(m);
    this.resize();
    this.setState("Starting indexer");
    this.host.post({ type: "ready" });
  }

  private initRenderer(): void {
    this.theme = readTheme();
    try {
      this.renderer = new GLRenderer(this.canvas);
    } catch (e) {
      this.host.post({ type: "log", level: "info", text: `WebGL2 unavailable, using Canvas2D: ${e}` });
      this.renderer = new Canvas2DRenderer(this.canvas);
    }
    this.renderer.setFont(this.theme.editorFont);
    this.widths = new WidthCache((t) => this.renderer.measure(t, 11));
  }

  private onThemeChange(): void {
    this.theme = readTheme();
    this.renderer.setFont(this.theme.editorFont);
    this.widths.clear();
    this.styleVersion++;
    this.invalidate();
  }

  private resize(): void {
    const r = this.root.getBoundingClientRect();
    const fit = this.camera.width <= 1;
    this.camera.resize(r.width, r.height);
    this.renderer.resize(r.width, r.height, window.devicePixelRatio || 1);
    this.renderer.setFont(this.theme.editorFont);
    if (fit && this.info) this.fit(false);
    this.updateZoomLimits();
    this.invalidate();
  }

  private updateZoomLimits(): void {
    if (!this.info) return;
    const fitScale = Math.min(this.camera.width / this.info.worldW, this.camera.height / this.info.worldH);
    this.camera.minScale = fitScale * 0.5;
    this.camera.maxScale = this.info.tilePx * 2 ** this.info.maxZoom * 4;
  }

  // ---------------------------------------------------------------- host messages

  private onHost(m: HostToView): void {
    switch (m.type) {
      case "status":
        if (m.state === "error") this.setState(m.text, "error");
        else if (!this.info) this.setState(m.text);
        if (m.state === "ready") void this.loadInfo();
        break;
      case "progress":
        // A finished phase shows nothing; incremental updates report progress without a
        // closing "idle", so completion is judged by the counts themselves.
        this.progress =
          m.phase === "idle" || m.done >= m.total ? "" : `${phaseLabel(m.phase)} ${m.done.toLocaleString("en-US")} / ${m.total.toLocaleString("en-US")} files`;
        if (!this.info || this.info.files === 0) {
          if (m.phase !== "idle") this.setState(`Indexing · ${this.progress}`);
        }
        this.updateLegend();
        if (m.phase === "idle") void this.loadInfo();
        break;
      case "info":
        this.applyInfo(m.info);
        break;
      case "changed":
        this.onChanged(m.layout);
        break;
      case "follow":
        this.onFollow(m.node);
        break;
      case "reveal":
        void this.reveal(m.slot, true);
        break;
      case "command":
        if (m.command === "fit") this.fit(true);
        else if (m.command === "toggleFollow") this.toggleFollow();
        else if (m.command === "search") this.search.show();
        break;
      case "response":
        break;
    }
  }

  private async loadInfo(): Promise<void> {
    try {
      this.applyInfo(await this.host.request("map/info", null));
    } catch (e) {
      this.setState(String(e), "error");
    }
  }

  private applyInfo(info: MapInfo): void {
    const first = !this.info;
    this.info = info;
    if (info.phase === "idle") this.progress = "";
    this.updateZoomLimits();
    if (first) {
      const saved = this.host.state.view as { cx: number; cy: number; scale: number } | undefined;
      if (saved) this.camera.jumpTo(saved);
      else this.fit(false);
    }
    if (info.files === 0) {
      if (info.phase === "idle") this.setState(`No files to map in ${info.rootName}.`);
    } else this.setState("");
    this.updateLegend();
    if (this.pendingFollow) this.onFollow(this.pendingFollow);
    this.invalidate();
  }

  private onChanged(layout: boolean): void {
    // Keep drawing the old tiles until fresh ones arrive, so the map never blanks. Without a
    // layout change only lens values and labels moved, but tiles carry those too.
    void layout;
    this.stale = this.cache;
    this.cache = new TileCache();
    this.edgeSets.clear();
    void this.loadInfo();
    this.invalidate();
  }

  private setState(text: string, kind: "info" | "error" = "info"): void {
    this.stateEl.textContent = text;
    this.stateEl.dataset.kind = kind;
    this.stateEl.hidden = !text;
  }

  private updateLegend(): void {
    const focus = this.selected ?? this.hover;
    const set = focus !== null ? this.edgeSets.get(focus) : undefined;
    let edges = "";
    if (set) {
      const out = set.edges.filter((e) => e.outgoing).length;
      const inn = set.edges.length - out;
      const more = set.total > set.edges.length ? ` (top ${set.edges.length} of ${set.total})` : "";
      edges = `purple uses ${out} · orange used by ${inn}${more} · dashed = name-matched`;
    }
    const status = [this.lensNotice, this.progress, edges, this.follow ? "" : "follow off"].filter(Boolean).join(" · ");
    this.legend.set(`${LENSES[this.lens].legend} · area = lines`, this.info?.files ?? 0, status);
  }

  // ---------------------------------------------------------------- tiles & scene

  private neededTiles(): { z: number; keys: { z: number; x: number; y: number }[] } {
    const info = this.info!;
    const z = Math.max(0, Math.min(info.maxZoom, Math.ceil(Math.log2(this.camera.view.scale / info.tilePx))));
    const n = 2 ** z;
    const b = this.camera.bounds();
    const x0 = Math.max(0, Math.floor(b.x * n));
    const y0 = Math.max(0, Math.floor(b.y * n));
    const x1 = Math.min(n - 1, Math.floor((b.x + b.w) * n));
    const y1 = Math.min(Math.ceil(info.worldH * n) - 1, Math.floor((b.y + b.h) * n));
    const keys = [];
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) keys.push({ z, x, y });
    return { z, keys };
  }

  private requestTiles(keys: { z: number; x: number; y: number }[]): void {
    const missing = keys.filter((k) => {
      const key = tileKey(k.z, k.x, k.y);
      return !this.cache.has(key) && !this.cache.isPending(key);
    });
    if (!missing.length) return;
    for (let i = 0; i < missing.length; i += MAX_TILE_REQUEST) {
      const chunk = missing.slice(i, i + MAX_TILE_REQUEST);
      for (const k of chunk) this.cache.markPending(tileKey(k.z, k.x, k.y));
      const cache = this.cache;
      this.host
        .request("map/tiles", { tiles: chunk })
        .then((r) => {
          for (const t of r.tiles as unknown as ViewTile[]) this.inbox.push({ cache, tile: t });
          this.invalidate();
        })
        .catch(() => {
          for (const k of chunk) cache.failed(tileKey(k.z, k.x, k.y));
        });
    }
  }

  /** Decodes queued tiles within the frame budget; returns true if work is left. */
  private drainInbox(t0: number): boolean {
    while (this.inbox.length) {
      const { cache, tile: t } = this.inbox.shift()!;
      try {
        cache.put(prepareTile(decodeTile(t.data)));
      } catch (e) {
        cache.failed(tileKey(t.z, t.x, t.y));
        this.host.post({ type: "log", level: "error", text: `tile ${t.z}/${t.x}/${t.y}: ${e}` });
      }
      if (performance.now() - t0 > DECODE_BUDGET_MS) break;
    }
    return this.inbox.length > 0;
  }

  private cached(key: string): DecodedTile | undefined {
    return this.cache.get(key) ?? this.stale.get(key);
  }

  /** Picks the best available tile for each needed key: exact, stale exact, or a cached ancestor. */
  private assemble(t0: number): TileDraw[] {
    this.drawn = [];
    if (!this.info) return [];
    const { keys } = this.neededTiles();
    this.requestTiles(keys);
    const style = this.baseStyle();
    const out: TileDraw[] = [];
    let deferred = false;
    /** A tile is usable if its batch is current, or can still be built within this frame's budget. */
    const ready = (t: DecodedTile | undefined): t is DecodedTile => {
      if (!t) return false;
      if (t.batch && t.batchVersion === this.styleVersion) return true;
      if (performance.now() - t0 > BUILD_BUDGET_MS) {
        deferred = true;
        return false;
      }
      t.batch = buildTileBatch(t, style);
      t.batchVersion = this.styleVersion;
      return true;
    };
    const use = (t: DecodedTile, z: number, x: number, y: number) => {
      const size = 1 / 2 ** z;
      const clip = { x: x * size, y: y * size, w: size, h: size };
      this.drawn.push({ tile: t, clip });
      out.push({ batch: t.batch!, clip });
    };
    for (const k of keys) {
      // Exact tile, else a coarser ancestor (zooming in), else finer children (zooming out).
      let t = this.cached(tileKey(k.z, k.x, k.y));
      if (!ready(t)) t = undefined;
      for (let d = 1; !t && d <= Math.min(k.z, 4); d++) {
        const a = this.cached(tileKey(k.z - d, k.x >> d, k.y >> d));
        if (ready(a)) t = a;
      }
      if (t) {
        use(t, k.z, k.x, k.y);
        continue;
      }
      for (let d = 1; d <= 2; d++) {
        const n = 1 << d;
        const kids: [DecodedTile, number, number][] = [];
        for (let dy = 0; dy < n; dy++)
          for (let dx = 0; dx < n; dx++) {
            const c = this.cached(tileKey(k.z + d, k.x * n + dx, k.y * n + dy));
            if (ready(c)) kids.push([c, k.x * n + dx, k.y * n + dy]);
          }
        if (kids.length) {
          for (const [c, x, y] of kids) use(c, k.z + d, x, y);
          break;
        }
      }
    }
    if (deferred) this.invalidate();
    return out;
  }

  private baseStyle(): BaseStyle {
    const lens = this.lens;
    const theme = this.theme;
    return { theme, lensColor: (it) => lensColor(lens, it, theme) };
  }

  /** Finds a visible item by slot (finest tile first). */
  private findItem(slot: number | null): Item | undefined {
    if (slot === null) return undefined;
    let best: Item | undefined;
    let bestZ = -1;
    for (const d of this.drawn) {
      const it = d.tile.bySlot?.get(slot);
      if (it && d.tile.z > bestZ) {
        best = it;
        bestZ = d.tile.z;
      }
    }
    return best;
  }

  /** Distinct visible items (for keyboard navigation). */
  private visibleItems(): Item[] {
    const seen = new Set<number>();
    const out: Item[] = [];
    for (const d of this.drawn) for (const it of d.tile.items) if (!seen.has(it.slot)) (seen.add(it.slot), out.push(it));
    return out;
  }

  /** Label candidates: per tile, largest first, stopping at items too small to label. */
  private labelCandidates(): Item[] {
    const seen = new Set<number>();
    const out: Item[] = [];
    const s2 = this.camera.view.scale ** 2;
    for (const d of this.drawn) {
      for (const it of d.tile.bySize ?? []) {
        if (it.w * it.h * s2 < 28 * 14) break;
        if (!seen.has(it.slot)) (seen.add(it.slot), out.push(it));
      }
    }
    return out;
  }

  private overlay(): Outline[] {
    const t = this.theme;
    const out: Outline[] = [];
    const add = (slot: number | null, color: Outline["color"], width: number) => {
      const it = this.findItem(slot);
      if (it) out.push({ it, color, width });
    };
    add(this.hover, t.muted, 1);
    for (const p of this.pinned) add(p, t.foreground, 1);
    add(this.here, t.accent, 2);
    add(this.selected, t.accent, 2);
    return out;
  }

  /** Nodes whose edges are shown: the selection (or the hovered node), plus pins. */
  private edgeSlots(): number[] {
    const out = new Set<number>(this.pinned);
    const focus = this.selected ?? this.hover;
    if (focus !== null) out.add(focus);
    return [...out];
  }

  private ensureEdges(slots: number[]): void {
    for (const slot of slots) {
      if (this.edgeSets.has(slot) || this.edgePending.has(slot)) continue;
      this.edgePending.add(slot);
      this.host
        .request("node/edges", { slot, limit: 300 })
        .then((r) => {
          this.edgeSets.set(slot, r);
          this.updateLegend();
          this.invalidate();
        })
        .catch(() => {})
        .finally(() => this.edgePending.delete(slot));
    }
  }

  /** Bundled edge curves in screen space. Purple: this node uses; orange: used by. */
  private edgeLines(): Line[] {
    const slots = this.edgeSlots();
    this.ensureEdges(slots);
    const t = this.theme;
    // Not blue: blue is too close to the accent, which only the selection may use.
    // Purple/orange stay distinct for the common colour-vision deficiencies.
    const total = slots.reduce((n, s) => n + (this.edgeSets.get(s)?.edges.length ?? 0), 0);
    // Many lines: lower opacity, so dense bundles read as volume, not noise.
    const alpha = Math.round(Math.max(70, Math.min(200, 200 * Math.sqrt(60 / Math.max(1, total)))));
    const uses = [...t.categorical[3]!.slice(0, 3), alpha] as Line["color"];
    const usedBy = [...t.categorical[1]!.slice(0, 3), alpha] as Line["color"];
    const v = this.camera.view;
    const hw = this.camera.width / 2;
    const hh = this.camera.height / 2;
    const out: Line[] = [];
    for (const slot of slots) {
      const set = this.edgeSets.get(slot);
      if (!set) continue;
      for (const e of set.edges) {
        const world = bundledCurve(e.route);
        if (world.length < 4) continue;
        const points = new Array<number>(world.length);
        for (let i = 0; i < world.length; i += 2) {
          points[i] = (world[i]! - v.cx) * v.scale + hw;
          points[i + 1] = (world[i + 1]! - v.cy) * v.scale + hh;
        }
        // Line style states certainty: dashed = name-matched, solid = confirmed, dotted = history.
        const style = e.confidence === "semantic" ? 0 : e.confidence === "historical" ? 2 : 1;
        out.push({ points, color: e.outgoing ? uses : usedBy, width: 1, style });
      }
    }
    return out;
  }

  invalidate(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame((now) => this.draw(now));
  }

  /** Frame log for performance tests: [rAF timestamp, CPU ms spent in draw]. */
  readonly frames: [number, number][] = [];
  /** Per-frame breakdown for profiling: [assemble, labels, render] in ms. */
  readonly breakdown: [number, number, number][] = [];

  private draw(now: number): void {
    const t0 = performance.now();
    this.frame = 0;
    const animating = this.camera.step(now);
    const moreToDecode = this.drainInbox(t0);
    const tiles = this.assemble(t0);
    const t1 = performance.now();
    const labels = this.info
      ? buildLabels(this.labelCandidates(), { camera: this.camera, theme: this.theme, measure: (t) => this.widths.width(t), max: 400 })
      : [];
    const t2 = performance.now();
    const lines = [...this.edgeLines(), ...hereMarker(this.findItem(this.here), this.camera, this.theme)];
    const b = this.camera.bounds();
    const outlines = this.overlay();
    const overlay = outlines.length ? buildOverlay(outlines, b.x, b.y, 256 / this.camera.view.scale) : null;
    this.renderer.draw({ view: this.camera.view, background: this.theme.background, tiles, overlay, labels, lines });
    const t3 = performance.now();
    this.frames.push([now, t3 - t0]);
    this.breakdown.push([t1 - t0, t2 - t1, t3 - t2]);
    if (this.breakdown.length > 2000) this.breakdown.splice(0, 1000);
    if (this.frames.length > 2000) this.frames.splice(0, 1000);
    if (animating || moreToDecode) this.invalidate();
    this.saveViewSoon();
  }

  private saveTimer = 0;
  private saveViewSoon(): void {
    clearTimeout(this.saveTimer);
    this.saveTimer = window.setTimeout(() => this.host.setState({ view: this.camera.view, follow: this.follow }), 300);
  }

  // ---------------------------------------------------------------- navigation

  private fit(animate: boolean): void {
    if (!this.info) return;
    const v = this.camera.fitView({ x: 0, y: 0, w: this.info.worldW, h: this.info.worldH }, 0.02);
    if (animate) this.camera.animateTo(v);
    else this.camera.jumpTo(v);
    this.invalidate();
  }

  private select(slot: number | null, recordHistory = true): void {
    if (slot === this.selected) return;
    if (recordHistory && this.selected !== null) {
      this.history.push(this.selected);
      if (this.history.length > 200) this.history.shift();
    }
    this.selected = slot;
    this.updateLegend();
    this.host.post({ type: "select", slot });
    this.invalidate();
  }

  /**
   * Test hook: drives the camera from requestAnimationFrame for `ms` (zoom in, pan, zoom out)
   * and returns the interval between frames, so tests measure rendering, not input delivery.
   */
  motionTest(ms: number): Promise<number[]> {
    return new Promise((resolve) => {
      const start = performance.now();
      let last = start;
      const intervals: number[] = [];
      const cx = this.camera.width / 2;
      const cy = this.camera.height / 2;
      const step = (now: number) => {
        intervals.push(now - last);
        last = now;
        const t = (now - start) / ms;
        if (t < 0.35) this.camera.zoomAt(cx + 120, cy - 60, 1.06);
        else if (t < 0.7) this.camera.panBy(14 * Math.sin(t * 20), 10 * Math.cos(t * 17));
        else this.camera.zoomAt(cx, cy, 1 / 1.06);
        if (this.frame) {
          cancelAnimationFrame(this.frame);
          this.frame = 0;
        }
        this.draw(now);
        if (t < 1) requestAnimationFrame(step);
        else resolve(intervals.slice(1));
      };
      requestAnimationFrame(step);
    });
  }

  private setLens(id: LensId): void {
    const def = LENSES[id];
    if (def.pending) {
      this.lensNotice = `${def.name} lens needs ${def.pending}`;
      this.updateLegend();
      window.setTimeout(() => {
        this.lensNotice = "";
        this.updateLegend();
      }, 2500);
      return;
    }
    if (id === this.lens) return;
    this.lens = id;
    this.styleVersion++;
    this.host.setState({ lens: id });
    this.updateLegend();
    this.invalidate();
  }

  /** Test hook: number of drawable items currently in the scene. */
  get itemCount(): number {
    return this.drawn.reduce((n, d) => n + d.tile.items.length, 0);
  }

  /** Selects a node and brings it into view (search results, sidebar, commands). */
  async reveal(slot: number, select: boolean): Promise<void> {
    try {
      const info = await this.host.request("node/info", { slot });
      if (select) this.select(slot);
      if (info.rect) this.showRect(info.rect, true);
    } catch {
      // Node vanished (file deleted) between search and reveal.
    }
  }

  /** Brings a rect into view if it is off-screen or too small to see. */
  private showRect(r: { x: number; y: number; w: number; h: number }, force: boolean): void {
    const b = this.camera.bounds();
    const px = Math.min(r.w, r.h) * this.camera.view.scale;
    const inside = r.x >= b.x && r.y >= b.y && r.x + r.w <= b.x + b.w && r.y + r.h <= b.y + b.h;
    if (!force && inside && px >= 24) return;
    // Show some context around the target: about four times its size.
    const k = 2;
    const target = { x: r.x - (r.w * (k - 1)) / 2, y: r.y - (r.h * (k - 1)) / 2, w: r.w * k, h: r.h * k };
    const fit = this.camera.fitView(target, 0.05);
    // Never zoom out to show a node that is already big enough and visible.
    if (inside && px >= 24) return;
    this.camera.animateTo(fit);
    this.invalidate();
  }

  private onFollow(node: NodeInfo | null): void {
    if (!this.info) {
      this.pendingFollow = node;
      return;
    }
    this.pendingFollow = null;
    this.here = node?.slot ?? null;
    if (this.follow && node?.rect) this.showRect(node.rect, false);
    this.invalidate();
  }

  private toggleFollow(): void {
    this.follow = !this.follow;
    this.host.post({ type: "follow", enabled: this.follow });
    this.updateLegend();
    this.saveViewSoon();
  }

  private hit(sx: number, sy: number): Item | undefined {
    const [wx, wy] = this.camera.toWorld(sx, sy);
    let best: Item | undefined;
    for (const d of this.drawn) {
      const c = d.clip;
      if (wx < c.x || wy < c.y || wx >= c.x + c.w || wy >= c.y + c.h) continue;
      for (const h of d.tile.items) {
        if (wx >= h.x && wx < h.x + h.w && wy >= h.y && wy < h.y + h.h && (!best || h.w * h.h < best.w * best.h)) best = h;
      }
    }
    return best;
  }

  /** Arrow-key navigation: nearest visible item of the same kind and depth in that direction. */
  private moveSelection(dx: number, dy: number): void {
    const items = this.visibleItems();
    const cur = this.findItem(this.selected);
    if (!cur) {
      const first = items.find((i) => i.kind === KIND_FILE) ?? items[0];
      if (first) this.select(first.slot);
      return;
    }
    const cx = cur.x + cur.w / 2;
    const cy = cur.y + cur.h / 2;
    let best: Item | undefined;
    let bestScore = Infinity;
    for (const it of items) {
      if (it.slot === cur.slot || it.kind !== cur.kind || it.depth !== cur.depth) continue;
      const vx = it.x + it.w / 2 - cx;
      const vy = it.y + it.h / 2 - cy;
      const along = vx * dx + vy * dy;
      if (along <= 0) continue;
      const across = Math.abs(vx * dy - vy * dx);
      const score = along + across * 2;
      if (score < bestScore) {
        bestScore = score;
        best = it;
      }
    }
    if (best) {
      this.select(best.slot);
      this.showRect(best, false);
    }
  }

  // ---------------------------------------------------------------- input

  private bindInput(): void {
    const c = this.canvas;
    let drag: { x: number; y: number; moved: boolean } | undefined;
    c.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        const r = c.getBoundingClientRect();
        const dy = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
        if (e.ctrlKey || !e.shiftKey) this.camera.zoomAt(e.clientX - r.left, e.clientY - r.top, Math.exp(-dy * 0.0022));
        else this.camera.panBy(-e.deltaX, -e.deltaY);
        this.invalidate();
      },
      { passive: false },
    );
    c.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      c.setPointerCapture(e.pointerId);
      drag = { x: e.clientX, y: e.clientY, moved: false };
      c.focus();
    });
    c.addEventListener("pointermove", (e) => {
      const r = c.getBoundingClientRect();
      if (drag) {
        const dx = e.clientX - drag.x;
        const dy = e.clientY - drag.y;
        if (drag.moved || Math.abs(dx) + Math.abs(dy) > 3) {
          drag.moved = true;
          this.camera.panBy(dx, dy);
          drag.x = e.clientX;
          drag.y = e.clientY;
          this.hoverCard.hide();
          this.invalidate();
          return;
        }
      }
      this.onHover(e.clientX - r.left, e.clientY - r.top, e.clientX, e.clientY);
    });
    c.addEventListener("pointerup", (e) => {
      const wasDrag = drag?.moved;
      drag = undefined;
      if (wasDrag) return;
      const r = c.getBoundingClientRect();
      const it = this.hit(e.clientX - r.left, e.clientY - r.top);
      this.select(it?.slot ?? null);
    });
    c.addEventListener("dblclick", (e) => {
      const r = c.getBoundingClientRect();
      const it = this.hit(e.clientX - r.left, e.clientY - r.top);
      if (it && it.kind !== KIND_DIR) this.host.post({ type: "open", slot: it.slot });
      else if (it) this.showRect(it, true);
    });
    c.addEventListener("pointerleave", () => {
      this.setHover(null);
      this.hoverCard.hide();
    });
    window.addEventListener("keydown", (e) => this.onKey(e));
  }

  private setHover(slot: number | null): void {
    if (slot === this.hover) return;
    this.hover = slot;
    if (this.selected === null) this.updateLegend();
    this.invalidate();
  }

  private onHover(sx: number, sy: number, cx: number, cy: number): void {
    const it = this.hit(sx, sy);
    this.setHover(it?.slot ?? null);
    clearTimeout(this.hoverTimer);
    if (!it) {
      this.hoverCard.hide();
      return;
    }
    const seq = ++this.hoverSeq;
    this.hoverTimer = window.setTimeout(async () => {
      try {
        const n = await this.host.request("node/info", { slot: it.slot });
        if (seq !== this.hoverSeq) return;
        this.hoverCard.show(hoverLines(n), cx, cy);
      } catch {
        this.hoverCard.hide();
      }
    }, 80);
  }

  private onKey(e: KeyboardEvent): void {
    if (this.search.open) return;
    if (e.target instanceof HTMLInputElement) return;
    const k = e.key;
    let handled = true;
    switch (k) {
      case "/":
        this.search.show();
        break;
      case "?":
        this.help.toggle();
        break;
      case "Escape":
        if (!this.help.el.hidden) this.help.toggle(false);
        else this.select(null);
        break;
      case "+":
      case "=":
        this.camera.animateTo({ ...this.camera.view, scale: this.camera.view.scale * 2 });
        break;
      case "-":
      case "_":
        this.camera.animateTo({ ...this.camera.view, scale: this.camera.view.scale / 2 });
        break;
      case "0":
        this.fit(true);
        break;
      case "1":
      case "2":
      case "3":
      case "4":
      case "5":
      case "6":
        this.setLens(Number(k) as LensId);
        break;
      case "f":
      case "F":
        this.toggleFollow();
        break;
      case "Enter":
        if (this.selected !== null) this.host.post({ type: "open", slot: this.selected });
        break;
      case "Backspace": {
        const prev = this.history.pop();
        if (prev !== undefined) {
          this.select(prev, false);
          const it = this.findItem(prev);
          if (it) this.showRect(it, false);
          else void this.reveal(prev, false);
        }
        break;
      }
      case ".":
        if (this.selected !== null) {
          if (this.pinned.has(this.selected)) this.pinned.delete(this.selected);
          else this.pinned.add(this.selected);
              }
        break;
      case "ArrowLeft":
        this.moveSelection(-1, 0);
        break;
      case "ArrowRight":
        this.moveSelection(1, 0);
        break;
      case "ArrowUp":
        this.moveSelection(0, -1);
        break;
      case "ArrowDown":
        this.moveSelection(0, 1);
        break;
      default:
        handled = false;
    }
    if (handled) {
      e.preventDefault();
      this.invalidate();
    }
  }
}

function phaseLabel(p: string): string {
  return p === "discover" ? "Discovering" : p === "hash" ? "Reading" : p === "parse" ? "Parsing" : p === "resolve" ? "Resolving" : p === "analyse" ? "Analysing" : p;
}

export function hoverLines(n: NodeInfo): { title: string; lines: string[] } {
  const loc = `${n.loc.toLocaleString("en-US")} lines`;
  const kind = n.kind === "file" ? (n.lang === "unknown" ? "file" : n.lang) : n.kind;
  const counted = n.data && n.areaLines < n.loc ? ` (counted as ${n.areaLines})` : "";
  const lines = [`${n.data ? "data" : kind} · ${loc}${counted}${n.generated ? " · generated" : ""}${n.test ? " · test" : ""}`];
  if (n.kind === "dir") lines[0] = `directory · ${n.files.toLocaleString("en-US")} files · ${loc}`;
  lines.push(n.kind === "file" || n.kind === "dir" ? n.path || "/" : `${n.path}:${n.startLine}`);
  if (n.inCount || n.outCount) lines.push(`used by ${n.inCount.toLocaleString("en-US")} · uses ${n.outCount.toLocaleString("en-US")}`);
  return { title: n.kind === "file" || n.kind === "dir" ? n.name : n.qualified, lines };
}

/** Indexes a decoded tile once: paint order, size order and slot lookup. */
export function prepareTile(t: DecodedTile): DecodedTile {
  sortForPaint(t.items);
  t.bySize = [...t.items].sort((a, b) => b.w * b.h - a.w * a.h);
  t.bySlot = new Map(t.items.map((i) => [i.slot, i]));
  return t;
}
