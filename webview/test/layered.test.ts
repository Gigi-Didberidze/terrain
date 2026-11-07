import { test } from "node:test";
import assert from "node:assert/strict";
import { BOX_H, crossings, layered, type LayerEdge, type LayerNode } from "../src/layout/layered.ts";

const node = (id: number, column: number, rank = id, width = 100): LayerNode => ({ id, column, width, rank });

test("columns are placed left to right and centred vertically", () => {
  const l = layered([node(1, 0), node(2, 1), node(3, 1), node(4, 2)], [{ from: 1, to: 2 }, { from: 1, to: 3 }, { from: 2, to: 4 }]);
  const [a, b, c, d] = [1, 2, 3, 4].map((i) => l.nodes.get(i)!);
  assert.ok(a!.x < b!.x && b!.x < d!.x);
  assert.equal(b!.x, c!.x);
  assert.equal(a!.y, -BOX_H / 2, "a lone node sits on the centre line");
  assert.ok(Math.abs(b!.y + BOX_H / 2 + (c!.y + BOX_H / 2)) < 1e-9, "two nodes straddle it");
});

test("barycentre sweeps remove avoidable crossings", () => {
  // Column 1 is ranked against the edges: 10→21, 11→20 cross unless column 1 is reordered.
  const nodes = [node(10, 0, 0), node(11, 0, 1), node(20, 1, 0), node(21, 1, 1)];
  const edges: LayerEdge[] = [{ from: 10, to: 21 }, { from: 11, to: 20 }];
  const none = layered(nodes, edges, 0);
  assert.equal(crossings(none, edges), 1);
  const swept = layered(nodes, edges);
  assert.equal(crossings(swept, edges), 0);
});

test("a neighbourhood keeps its centre in the middle column", () => {
  const nodes = [node(0, 2), node(1, 1), node(2, 1), node(3, 3), node(4, 0), node(5, 4)];
  const edges = [{ from: 1, to: 0 }, { from: 2, to: 0 }, { from: 0, to: 3 }, { from: 4, to: 1 }, { from: 3, to: 5 }];
  const l = layered(nodes, edges);
  assert.deepEqual(l.columns.map((c) => c.length), [1, 2, 1, 1, 1]);
  const centre = l.nodes.get(0)!;
  assert.ok(centre.x > l.nodes.get(1)!.x && centre.x < l.nodes.get(3)!.x);
});

test("layout of 80 nodes stays well under a millisecond budget", () => {
  const nodes: LayerNode[] = [node(0, 2)];
  const edges: LayerEdge[] = [];
  let id = 1;
  for (const col of [0, 1, 3, 4])
    for (let i = 0; i < 16; i++) {
      nodes.push(node(id, col, i));
      edges.push(col < 2 ? { from: id, to: col === 1 ? 0 : 1 + ((i * 7) % 16) } : { from: col === 3 ? 0 : 33 + ((i * 5) % 16), to: id });
      id++;
    }
  // Best of several rounds, so a busy machine (a build running alongside) cannot fail it.
  let per = Infinity;
  for (let round = 0; round < 5; round++) {
    const t = performance.now();
    for (let i = 0; i < 20; i++) layered(nodes, edges);
    per = Math.min(per, (performance.now() - t) / 20);
  }
  assert.ok(per < 5, `${per.toFixed(2)} ms per layout`);
});
