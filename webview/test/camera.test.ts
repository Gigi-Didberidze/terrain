import { test } from "node:test";
import assert from "node:assert/strict";
import { Camera, TRANSITION_MS } from "../src/camera.ts";

test("zooming keeps the point under the cursor fixed", () => {
  const c = new Camera();
  c.resize(800, 500);
  c.jumpTo({ cx: 0.5, cy: 0.3, scale: 1000 });
  const before = c.toWorld(123, 456);
  c.zoomAt(123, 456, 3.7);
  const after = c.toWorld(123, 456);
  assert.ok(Math.abs(before[0] - after[0]) < 1e-12 && Math.abs(before[1] - after[1]) < 1e-12);
});

test("fit keeps the rect above the bottom inset", () => {
  const c = new Camera();
  c.resize(1000, 600);
  c.bottomInset = 24;
  c.jumpTo(c.fitView({ x: 0, y: 0, w: 1, h: 0.625 }, 0));
  const [, bottom] = c.toScreen(0, 0.625);
  assert.ok(bottom <= 600 - 24 + 1e-6);
});

test("animations finish in 150 ms and land exactly", () => {
  const c = new Camera();
  c.resize(800, 500);
  c.jumpTo({ cx: 0, cy: 0, scale: 1000 });
  c.animateTo({ cx: 1, cy: 1, scale: 4000 }, 0);
  assert.ok(c.step(TRANSITION_MS / 2));
  c.step(TRANSITION_MS);
  assert.deepEqual(c.view, { cx: 1, cy: 1, scale: 4000 });
  assert.equal(c.animating, false);
});
