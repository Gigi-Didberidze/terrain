import { test } from "node:test";
import assert from "node:assert/strict";
import { neighbourhoodModel, pathModel } from "../src/graph.ts";
import type { NeighbourhoodResult, NodeRef } from "../../protocol/generated/protocol.ts";

const ref = (slot: number, name = `n${slot}`): NodeRef => ({ slot, name, kind: "function", path: "a.ts", line: 1 });
const nb = (slot: number, column: number) => ({ node: ref(slot), column, load: 0, inCount: 0, outCount: 0 });

test("neighbourhood edges left of the centre read as 'used by', right of it as 'uses'", () => {
  const r: NeighbourhoodResult = {
    centre: 0,
    nodes: [nb(0, 2), nb(1, 1), nb(2, 0), nb(3, 3), nb(4, 4)],
    edges: [
      { from: 2, to: 1, kind: "calls", confidence: "syntactic", count: 1 },
      { from: 1, to: 0, kind: "calls", confidence: "semantic", count: 1 },
      { from: 0, to: 3, kind: "calls", confidence: "syntactic", count: 1 },
      { from: 3, to: 4, kind: "imports", confidence: "historical", count: 1 },
    ],
    dependents: 40,
    dependencies: 1,
    millis: 1,
  };
  const m = neighbourhoodModel(r);
  assert.deepEqual(m.edges.map((e) => e.usedBy), [true, true, false, false]);
  assert.deepEqual(m.edges.map((e) => e.style), [1, 0, 1, 2]);
  assert.equal(m.captions.get(1), "used by · 1 of 40");
  assert.equal(m.captions.get(3), "uses · 1");
});

test("path model puts both ends in fixed columns and merges shared nodes", () => {
  const m = pathModel({
    from: ref(1, "a"),
    to: ref(9, "z"),
    reversed: false,
    paths: [{ nodes: [ref(1, "a"), ref(2), ref(9, "z")] }, { nodes: [ref(1, "a"), ref(3), ref(4), ref(9, "z")] }],
  });
  const col = new Map(m.nodes.map((n) => [n.slot, n.column]));
  assert.equal(col.get(1), 0);
  assert.equal(col.get(9), 3, "the target shares the last column");
  assert.equal(m.edges.length, 5);
  assert.match(m.summary, /2 shortest chains: a uses z in 2 steps/);
});
