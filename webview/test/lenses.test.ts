import { test } from "node:test";
import assert from "node:assert/strict";
import { couplingMuted, lensColor, shortAuthor, type CouplingFocus } from "../src/lenses.ts";
import type { RGBA, Theme } from "../src/render/theme.ts";
import { KIND_DIR, KIND_FILE, type Item } from "../src/tiles.ts";

const c = (v: number): RGBA => [v, v, v, 255];
const theme: Theme = {
  kind: "dark",
  background: c(0),
  foreground: c(255),
  muted: c(150),
  line: c(60),
  accent: [0, 0, 255, 255],
  dirFill: c(5),
  fileFill: c(18),
  symbolFill: c(30),
  generatedFill: c(9),
  categorical: [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], c(1), c(2), c(3), c(4), c(5)],
  rampHigh: [0, 0, 255, 255],
  churnHigh: [255, 128, 0, 255],
  coupling: [255, 255, 0, 255],
  hiddenCoupling: [255, 0, 0, 255],
  changed: [255, 128, 0, 255],
  editorFont: "monospace",
  uiFont: "sans-serif",
};

const KIND_FUNCTION = 4;
const item = (slot: number, kind: number, x: number, y: number, w: number, h: number, extra: Partial<Item> = {}): Item => ({
  slot, kind, depth: 0, x, y, w, h, flags: 0, hidden: 0, label: "", community: 0, owner: 0, load: 0, churn: 0, ...extra,
});

test("churn lens leaves untouched files neutral and ramps the rest", () => {
  assert.equal(lensColor(3, item(1, KIND_FILE, 0, 0, 1, 1), theme), null);
  const low = lensColor(3, item(1, KIND_FILE, 0, 0, 1, 1, { churn: 1000 }), theme)!;
  const high = lensColor(3, item(1, KIND_FILE, 0, 0, 1, 1, { churn: 65535 }), theme)!;
  assert.ok(high[0] > low[0], "busier files are closer to the ramp end");
  assert.equal(lensColor(3, item(1, KIND_DIR, 0, 0, 1, 1, { churn: 65535 }), theme), null);
});

test("ownership lens colours by author index", () => {
  const first = lensColor(5, item(1, KIND_FILE, 0, 0, 1, 1, { owner: 1 }), theme)!;
  const second = lensColor(5, item(1, KIND_FILE, 0, 0, 1, 1, { owner: 2 }), theme)!;
  assert.ok(first[0] > first[1] && second[1] > second[0]);
  assert.equal(lensColor(5, item(1, KIND_FILE, 0, 0, 1, 1), theme), null);
});

test("coupling lens marks partners, hidden ones in their own colour, and mutes the rest", () => {
  const focus: CouplingFocus = {
    file: 1,
    fileRect: { x: 0, y: 0, w: 0.2, h: 0.2 },
    partners: new Map([
      [2, { slot: 2, degree: 0.9, hidden: true, rect: { x: 0.5, y: 0.5, w: 0.2, h: 0.2 } }],
      [3, { slot: 3, degree: 0.9, hidden: false, rect: { x: 0.8, y: 0, w: 0.1, h: 0.1 } }],
    ]),
  };
  const hidden = lensColor(4, item(2, KIND_FILE, 0.5, 0.5, 0.2, 0.2), theme, focus)!;
  const linked = lensColor(4, item(3, KIND_FILE, 0.8, 0, 0.1, 0.1), theme, focus)!;
  assert.ok(hidden[1] < linked[1], "hidden partners lean red, linked ones yellow");
  // A symbol inside a partner file takes the partner's colour; one elsewhere is muted.
  const inPartner = item(20, KIND_FUNCTION, 0.55, 0.55, 0.05, 0.05);
  assert.notEqual(lensColor(4, inPartner, theme, focus), null);
  assert.equal(couplingMuted(inPartner, focus), false);
  assert.equal(couplingMuted(item(9, KIND_FILE, 0.3, 0.3, 0.1, 0.1), focus), true);
  // The selected file and its symbols stay as they are.
  assert.equal(couplingMuted(item(1, KIND_FILE, 0, 0, 0.2, 0.2), focus), false);
  assert.equal(couplingMuted(item(11, KIND_FUNCTION, 0.05, 0.05, 0.05, 0.05), focus), false);
  // Directories and no selection: nothing muted.
  assert.equal(couplingMuted(item(0, KIND_DIR, 0, 0, 1, 1), focus), false);
  assert.equal(couplingMuted(item(9, KIND_FILE, 0.3, 0.3, 0.1, 0.1), null), false);
});

test("author names drop the email domain and GitHub's numeric noreply prefix", () => {
  assert.equal(shortAuthor("ann@example.com"), "ann");
  assert.equal(shortAuthor("112854064+shimonrm@users.noreply.github.com"), "shimonrm");
  assert.equal(shortAuthor("Ann Smith"), "Ann Smith");
});
