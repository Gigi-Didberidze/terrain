// The map view: tile streaming, scene assembly, input and the render loop.
// Frames are drawn only when something changes; nothing animates on its own.
import type { MapInfo, NodeInfo } from "../../protocol/generated/protocol.ts";
import type { HostToView, ViewTile } from "../../protocol/webview.ts";
import { Camera } from "./camera.ts";
import { Host } from "./host.ts";
import { Quadtree } from "./layout/quadtree.ts";
import { Canvas2DRenderer } from "./render/canvas2d.ts";
import { GLRenderer } from "./render/gl.ts";
import { readTheme, type Theme } from "./render/theme.ts";
import { emptyBatch, type RectBatch, type Renderer } from "./render/types.ts";
import { buildBatch, buildLabels, hereMarker, sortForPaint, type Style } from "./scene.ts";
import { decodeTile, KIND_DIR, KIND_FILE, TileCache, tileKey, type Item } from "./tiles.ts";
import { Help } from "./ui/help.ts";
import { HoverCard } from "./ui/hover.ts";
import { Legend } from "./ui/legend.ts";
import { SearchBox } from "./ui/search.ts";

const MAX_ITEMS = 50_000;
const MAX_TILE_REQUEST = 64;

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
  private items: Item[] = [];
  private byslot = new Map<number, Item>();
  private quad = new Quadtree<Item>();
  private batch: RectBatch = emptyBatch();
  private sceneKey = "";
  private styleDirty = true;
  private frame = 0;
  private origin = { x: 0, y: 0, unit: 1 };

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
  }

  private onThemeChange(): void {
    this.theme = readTheme();
    this.renderer.setFont(this.theme.editorFont);
    this.styleDirty = true;
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
        this.progress = m.phase === "idle" ? "" : `${phaseLabel(m.phase)} ${m.done.toLocaleString("en-US")} / ${m.total.toLocaleString("en-US")} files`;
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
    void this.loadInfo();
    this.invalidate();
  }

  private setState(text: string, kind: "info" | "error" = "info"): void {
    this.stateEl.textContent = text;
    this.stateEl.dataset.kind = kind;
    this.stateEl.hidden = !text;
  }

  private updateLegend(): void {
    this.legend.set("Area = lines of code", this.info?.files ?? 0, this.progress || (this.follow ? "" : "follow off"));
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
          for (const t of r.tiles as unknown as ViewTile[]) {
            try {
              cache.put(decodeTile(t.data));
            } catch (e) {
              cache.failed(tileKey(t.z, t.x, t.y));
              this.host.post({ type: "log", level: "error", text: `tile ${t.z}/${t.x}/${t.y}: ${e}` });
            }
          }
          this.invalidate();
        })
        .catch(() => {
          for (const k of chunk) cache.failed(tileKey(k.z, k.x, k.y));
        });
    }
  }

  /** Picks the best available tile for each needed key: exact, stale exact, or a cached ancestor. */
  private assemble(): void {
    if (!this.info) return;
    const { keys } = this.neededTiles();

    this.requestTiles(keys);
    const used: string[] = [];
    const tiles = [];
    const seen = new Set<string>();
    for (const k of keys) {
      let t = this.cache.get(tileKey(k.z, k.x, k.y)) ?? this.stale.get(tileKey(k.z, k.x, k.y));
      for (let d = 1; !t && d <= Math.min(k.z, 4); d++) {
        const pk = tileKey(k.z - d, k.x >> d, k.y >> d);
        t = this.cache.get(pk) ?? this.stale.get(pk);
      }
      if (!t) continue;
      const key = tileKey(t.z, t.x, t.y);
      if (seen.has(key)) continue;
      seen.add(key);
      used.push(key);
      tiles.push(t);
    }
    const sceneKey = used.sort().join("|");
    // Rebase when zoom moved far from the batch origin (keeps f32 precise at deep zoom).
    const unitOk = this.origin.unit * this.camera.view.scale > 64 && this.origin.unit * this.camera.view.scale < 1 << 16;
    if (sceneKey === this.sceneKey && !this.styleDirty && unitOk) return;
    if (sceneKey !== this.sceneKey) {
      const map = new Map<number, Item>();
      for (const t of tiles) for (const it of t.items) if (!map.has(it.slot)) map.set(it.slot, it);
      let items = [...map.values()];
      if (items.length > MAX_ITEMS) items = items.sort((a, b) => b.w * b.h - a.w * a.h).slice(0, MAX_ITEMS);
      this.items = sortForPaint(items);
      this.byslot = map;
      const q = new Quadtree<Item>({ x: 0, y: 0, w: 1, h: 1 });
      for (const it of this.items) q.insert(it);
      this.quad = q;
      this.sceneKey = sceneKey;
    }
    // Rebase on every rebuild: origin at the viewport corner, 256 px per local unit.
    const b = this.camera.bounds();
    this.origin = { x: b.x, y: b.y, unit: 256 / this.camera.view.scale };
    this.batch = buildBatch(this.items, this.style(), this.origin.x, this.origin.y, this.origin.unit, this.batch);
    this.renderer.setRects(this.batch);
    this.styleDirty = false;
  }

  private style(): Style {
    return {
      theme: this.theme,
      selected: this.selected,
      here: this.here,
      hover: this.hover,
      pinned: this.pinned,
      lensColor: () => null,
    };
  }

  invalidate(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame((now) => this.draw(now));
  }

  /** Frame log for performance tests: [rAF timestamp, CPU ms spent in draw]. */
  readonly frames: [number, number][] = [];

  private draw(now: number): void {
    const t0 = performance.now();
    this.frame = 0;
    const animating = this.camera.step(now);
    if (this.info) this.assemble();
    const labels = this.info
      ? buildLabels(this.items, { camera: this.camera, theme: this.theme, measure: (t, s) => this.renderer.measure(t, s), max: 800 })
      : [];
    const lines = hereMarker(this.here !== null ? this.byslot.get(this.here) : undefined, this.camera, this.theme);
    this.renderer.draw(this.camera.view, this.theme.background, labels, lines);
    this.frames.push([now, performance.now() - t0]);
    if (this.frames.length > 2000) this.frames.splice(0, 1000);
    if (animating) this.invalidate();
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
    this.styleDirty = true;
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

  /** Test hook: number of drawable items currently in the scene. */
  get itemCount(): number {
    return this.items.length;
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
    this.styleDirty = true;
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
    const hits = this.quad.at(wx, wy);
    let best: Item | undefined;
    for (const h of hits) if (!best || h.w * h.h < best.w * best.h) best = h;
    return best;
  }

  /** Arrow-key navigation: nearest visible item of the same kind and depth in that direction. */
  private moveSelection(dx: number, dy: number): void {
    const cur = this.selected !== null ? this.byslot.get(this.selected) : undefined;
    if (!cur) {
      const first = this.items.find((i) => i.kind === KIND_FILE) ?? this.items[0];
      if (first) this.select(first.slot);
      return;
    }
    const cx = cur.x + cur.w / 2;
    const cy = cur.y + cur.h / 2;
    let best: Item | undefined;
    let bestScore = Infinity;
    for (const it of this.items) {
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
    this.styleDirty = true;
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
          const it = this.byslot.get(prev);
          if (it) this.showRect(it, false);
          else void this.reveal(prev, false);
        }
        break;
      }
      case ".":
        if (this.selected !== null) {
          if (this.pinned.has(this.selected)) this.pinned.delete(this.selected);
          else this.pinned.add(this.selected);
          this.styleDirty = true;
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
  const lines = [`${kind} · ${loc}${n.generated ? " · generated" : ""}${n.test ? " · test" : ""}`];
  if (n.kind === "dir") lines[0] = `directory · ${n.files.toLocaleString("en-US")} files · ${loc}`;
  lines.push(n.kind === "file" || n.kind === "dir" ? n.path || "/" : `${n.path}:${n.startLine}`);
  return { title: n.kind === "file" || n.kind === "dir" ? n.name : n.qualified, lines };
}
