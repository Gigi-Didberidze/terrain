// Quadtree over item rectangles (world coordinates) for hit-testing and culling.

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

const MAX_ITEMS = 16;
const MAX_DEPTH = 24;

class QNode<T extends Box> {
  readonly items: T[] = [];
  kids: QNode<T>[] | undefined;
  readonly b: Box;
  readonly depth: number;
  constructor(b: Box, depth: number) {
    this.b = b;
    this.depth = depth;
  }

  insert(it: T): void {
    if (this.kids) {
      const k = this.childFor(it);
      if (k) return k.insert(it);
      this.items.push(it);
      return;
    }
    this.items.push(it);
    if (this.items.length > MAX_ITEMS && this.depth < MAX_DEPTH) this.split();
  }

  private split(): void {
    const { x, y, w, h } = this.b;
    const hw = w / 2;
    const hh = h / 2;
    this.kids = [
      new QNode<T>({ x, y, w: hw, h: hh }, this.depth + 1),
      new QNode<T>({ x: x + hw, y, w: hw, h: hh }, this.depth + 1),
      new QNode<T>({ x, y: y + hh, w: hw, h: hh }, this.depth + 1),
      new QNode<T>({ x: x + hw, y: y + hh, w: hw, h: hh }, this.depth + 1),
    ];
    const old = this.items.splice(0);
    for (const it of old) this.insert(it);
  }

  /** Child that fully contains the item, if any. */
  private childFor(it: Box): QNode<T> | undefined {
    for (const k of this.kids!) {
      const b = k.b;
      if (it.x >= b.x && it.y >= b.y && it.x + it.w <= b.x + b.w && it.y + it.h <= b.y + b.h) return k;
    }
    return undefined;
  }

  queryPoint(px: number, py: number, out: T[]): void {
    for (const it of this.items) if (px >= it.x && px < it.x + it.w && py >= it.y && py < it.y + it.h) out.push(it);
    if (!this.kids) return;
    for (const k of this.kids) {
      const b = k.b;
      if (px >= b.x && px < b.x + b.w && py >= b.y && py < b.y + b.h) k.queryPoint(px, py, out);
    }
  }

  queryBox(q: Box, out: T[]): void {
    for (const it of this.items) if (it.x < q.x + q.w && q.x < it.x + it.w && it.y < q.y + q.h && q.y < it.y + it.h) out.push(it);
    if (!this.kids) return;
    for (const k of this.kids) {
      const b = k.b;
      if (b.x < q.x + q.w && q.x < b.x + b.w && b.y < q.y + q.h && q.y < b.y + b.h) k.queryBox(q, out);
    }
  }
}

export class Quadtree<T extends Box> {
  private root: QNode<T>;
  size = 0;
  constructor(bounds: Box = { x: 0, y: 0, w: 1, h: 1 }) {
    this.root = new QNode<T>(bounds, 0);
  }
  insert(it: T): void {
    this.root.insert(it);
    this.size++;
  }
  /** All items containing the point. */
  at(x: number, y: number): T[] {
    const out: T[] = [];
    this.root.queryPoint(x, y, out);
    return out;
  }
  within(b: Box): T[] {
    const out: T[] = [];
    this.root.queryBox(b, out);
    return out;
  }
}
