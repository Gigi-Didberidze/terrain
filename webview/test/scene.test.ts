import { test } from "node:test";
import assert from "node:assert/strict";
import { fitText, sortForPaint } from "../src/scene.ts";
import { Quadtree } from "../src/layout/quadtree.ts";
import type { Item } from "../src/tiles.ts";

const mono = (t: string) => t.length * 7;

test("fitText truncates with an ellipsis and refuses tiny fits", () => {
  assert.equal(fitText("arrays.ts", 100, mono), "arrays.ts");
  assert.equal(fitText("arraysFind.ts", 50, mono), "arrays…");
  assert.equal(fitText("arraysFind.ts", 20, mono), null);
});

const item = (slot: number, kind: number, depth: number, x = 0, y = 0, w = 1, h = 1): Item => ({
  slot, kind, depth, x, y, w, h, flags: 0, hidden: 0, label: "", community: 0, owner: 0, load: 0, churn: 0,
});

test("paint order puts containers before contents", () => {
  const order = sortForPaint([item(1, 4, 3), item(2, 1, 2), item(3, 0, 1), item(4, 0, 0)]).map((i) => i.slot);
  assert.deepEqual(order, [4, 3, 2, 1]);
});

test("quadtree finds every item containing a point", () => {
  const q = new Quadtree<Item>();
  const items: Item[] = [];
  for (let i = 0; i < 500; i++) {
    const it = item(i, 1, 0, (i % 25) / 25, Math.floor(i / 25) / 25, 1 / 25, 1 / 25);
    items.push(it);
    q.insert(it);
  }
  const big = item(999, 0, 0, 0, 0, 1, 1);
  q.insert(big);
  for (const [px, py] of [[0.01, 0.01], [0.5, 0.5], [0.99, 0.75]] as const) {
    const expect = [...items, big].filter((i) => px >= i.x && px < i.x + i.w && py >= i.y && py < i.y + i.h).map((i) => i.slot).sort();
    assert.deepEqual(q.at(px, py).map((i) => i.slot).sort(), expect);
  }
});
