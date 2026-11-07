import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeTile, HEADER_BYTES, ITEM_BYTES, MAGIC, TileCache } from "../src/tiles.ts";

/** Builds a tile buffer the same way indexer/crates/core/src/tiles.rs packs it. */
function pack(z: number, x: number, y: number, items: { rx: number; ry: number; rw: number; rh: number; slot: number; label: string; community?: number }[]): Uint8Array {
  const enc = new TextEncoder();
  const labels = items.map((i) => enc.encode(i.label));
  const slen = labels.reduce((a, l) => a + l.length, 0);
  const buf = new Uint8Array(HEADER_BYTES + items.length * ITEM_BYTES + slen);
  const dv = new DataView(buf.buffer);
  dv.setUint32(0, MAGIC, true);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, z, true);
  dv.setUint32(8, x, true);
  dv.setUint32(12, y, true);
  dv.setUint32(16, items.length, true);
  dv.setUint32(20, slen, true);
  let off = 0;
  items.forEach((it, i) => {
    const o = HEADER_BYTES + i * ITEM_BYTES;
    dv.setFloat32(o, it.rx, true);
    dv.setFloat32(o + 4, it.ry, true);
    dv.setFloat32(o + 8, it.rw, true);
    dv.setFloat32(o + 12, it.rh, true);
    dv.setUint32(o + 16, it.slot, true);
    dv.setUint8(o + 20, 1);
    dv.setUint32(o + 28, off, true);
    dv.setUint16(o + 32, labels[i]!.length, true);
    dv.setUint8(o + 34, it.community ?? 0);
    buf.set(labels[i]!, HEADER_BYTES + items.length * ITEM_BYTES + off);
    off += labels[i]!.length;
  });
  return buf;
}

test("decodes items into world coordinates", () => {
  const t = decodeTile(pack(2, 1, 3, [{ rx: 0.5, ry: 0.25, rw: 0.25, rh: 0.5, slot: 42, label: "arrays.ts", community: 7 }]));
  const it = t.items[0]!;
  assert.equal(it.slot, 42);
  assert.equal(it.label, "arrays.ts");
  assert.equal(it.community, 7);
  assert.ok(Math.abs(it.x - (1 + 0.5) / 4) < 1e-9);
  assert.ok(Math.abs(it.y - (3 + 0.25) / 4) < 1e-9);
  assert.ok(Math.abs(it.w - 0.25 / 4) < 1e-9);
});

test("decodes multi-byte labels", () => {
  const t = decodeTile(pack(0, 0, 0, [{ rx: 0, ry: 0, rw: 1, rh: 1, slot: 1, label: "⊗.txt" }, { rx: 0, ry: 0, rw: 1, rh: 1, slot: 2, label: "b" }]));
  assert.deepEqual(t.items.map((i) => i.label), ["⊗.txt", "b"]);
});

test("rejects foreign buffers", () => {
  assert.throws(() => decodeTile(new Uint8Array(32)));
});

test("tile cache evicts least recently used", () => {
  const c = new TileCache(2);
  c.put({ z: 0, x: 0, y: 0, items: [] });
  c.put({ z: 1, x: 0, y: 0, items: [] });
  c.get("0/0/0");
  c.put({ z: 1, x: 1, y: 0, items: [] });
  assert.ok(c.has("0/0/0"));
  assert.ok(!c.has("1/0/0"));
});

test("tile magic matches the indexer's (the bytes \"TRRN\", read little-endian)", () => {
  // The indexer writes MAGIC = u32::from_le_bytes(*b"TRRN"); a stale number here makes every
  // tile fail to decode, which only the visual tests would otherwise notice.
  const bytes = new Uint8Array(new Uint32Array([MAGIC]).buffer);
  assert.equal(String.fromCharCode(...bytes), "TRRN");
});
