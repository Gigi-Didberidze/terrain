import { test } from "node:test";
import assert from "node:assert/strict";
import { frameToSvg } from "../src/export/svg.ts";
import type { Frame } from "../src/render/types.ts";

test("SVG export clips rectangles to their tile and escapes labels", () => {
  const frame: Frame = {
    view: { cx: 0.5, cy: 0.5, scale: 100 },
    background: [30, 30, 30, 255],
    tiles: [
      {
        // Two rects in a tile covering world x 0..0.5; the second sticks out of it.
        batch: {
          originX: 0,
          originY: 0,
          unit: 1,
          count: 2,
          geom: new Float32Array([0, 0, 0.25, 0.25, 0.4, 0, 0.3, 0.1]),
          colors: new Uint8Array([10, 20, 30, 255, 1, 2, 3, 255, 40, 50, 60, 255, 0, 0, 0, 0]),
          stroke: new Float32Array([1, 0]),
        },
        clip: { x: 0, y: 0, w: 0.5, h: 1 },
      },
    ],
    overlay: null,
    labels: [{ text: "a<b & c", x: 5, y: 5, color: [200, 200, 200, 255], size: 11 }],
    lines: [{ points: [0, 0, 10, 10], color: [1, 2, 3, 128], width: 1, style: 1 }],
  };
  const svg = frameToSvg(frame, 100, 100, "Menlo");
  assert.match(svg, /^<svg xmlns="http:\/\/www.w3.org\/2000\/svg" width="100" height="100"/);
  assert.match(svg, /<rect x="0" y="0" width="25" height="25" fill="rgb\(10,20,30\)" stroke="rgb\(1,2,3\)" stroke-width="1"\/>/);
  // 0.4..0.7 clipped to the tile's right edge at 0.5 → 10 px wide.
  assert.match(svg, /<rect x="40" y="0" width="10" height="10" fill="rgb\(40,50,60\)"\/>/);
  assert.match(svg, />a&lt;b &amp; c<\/text>/);
  assert.match(svg, /stroke-dasharray="5 3"/);
  assert.match(svg, /stroke="rgba\(1,2,3,0.502\)"/);
});
