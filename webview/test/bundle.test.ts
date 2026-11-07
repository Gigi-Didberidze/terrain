import { test } from "node:test";
import assert from "node:assert/strict";
import { bundledCurve } from "../src/layout/bundle.ts";

const box = (cx: number, cy: number) => ({ x: cx - 0.5, y: cy - 0.5, w: 1, h: 1 });

test("curve starts and ends at the endpoint centres", () => {
  const pts = bundledCurve([box(0, 0), box(5, 5), box(10, 0)]);
  assert.deepEqual(pts.slice(0, 2).map((v) => +v.toFixed(9)), [0, 0]);
  assert.deepEqual(pts.slice(-2), [10, 0]);
});

test("beta 0 is a straight line, beta 1 bends towards the hierarchy", () => {
  const route = [box(0, 0), box(5, 10), box(10, 0)];
  const straight = bundledCurve(route, 0);
  for (let i = 1; i < straight.length; i += 2) assert.ok(Math.abs(straight[i]!) < 1e-9);
  const bent = bundledCurve(route, 1);
  assert.ok(Math.max(...bent.filter((_, i) => i % 2 === 1)) > 3);
});

test("edges through the same containers share a route", () => {
  const a = bundledCurve([box(0, 0), box(5, 8), box(10, 0)]);
  const b = bundledCurve([box(0, 1), box(5, 8), box(10, 1)]);
  const mid = (p: number[]) => [p[p.length / 2 - 1]!, p[p.length / 2]!];
  const [ma, mb] = [mid(a), mid(b)];
  assert.ok(Math.hypot(ma[0]! - mb[0]!, ma[1]! - mb[1]!) < 1);
});
