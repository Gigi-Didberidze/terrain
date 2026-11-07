// Binary tile decoding (format v1, see indexer/crates/core/src/tiles.rs) and the tile cache.

export const MAGIC = 0x4e525254; // "TRRN" little-endian
export const HEADER_BYTES = 24;
export const ITEM_BYTES = 40;

export const KIND_DIR = 0;
export const KIND_FILE = 1;
export const FLAG_GENERATED = 1 << 2;
export const FLAG_TEST = 1 << 3;
export const FLAG_DATA = 1 << 7;
/** Tile-only flags (Change lens): changed on this branch, or depends on something changed. */
export const FLAG_CHANGED = 1 << 14;
export const FLAG_BLAST = 1 << 15;

/** One drawable node, in world coordinates (f64). */
export interface Item {
  slot: number;
  x: number;
  y: number;
  w: number;
  h: number;
  kind: number;
  depth: number;
  flags: number;
  hidden: number;
  label: string;
  community: number;
  owner: number;
  load: number;
  churn: number;
}

export interface DecodedTile {
  z: number;
  x: number;
  y: number;
  /** In paint order (containers before contents). */
  items: Item[];
  /** Largest first, for label placement. */
  bySize?: Item[];
  bySlot?: Map<number, Item>;
  /** GPU batch and the style version it was built for. */
  batch?: import("./render/types.ts").RectBatch;
  batchVersion?: number;
}

const decoder = new TextDecoder();

export function decodeTile(buf: Uint8Array): DecodedTile {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  if (dv.getUint32(0, true) !== MAGIC) throw new Error("bad tile magic");
  const version = dv.getUint16(4, true);
  if (version !== 1) throw new Error(`unsupported tile version ${version}`);
  const z = dv.getUint16(6, true);
  const tx = dv.getUint32(8, true);
  const ty = dv.getUint32(12, true);
  const count = dv.getUint32(16, true);
  const stringsAt = HEADER_BYTES + count * ITEM_BYTES;
  const size = 1 / 2 ** z;
  const ox = tx * size;
  const oy = ty * size;
  const items: Item[] = new Array(count);
  for (let i = 0; i < count; i++) {
    const o = HEADER_BYTES + i * ITEM_BYTES;
    const labelOff = dv.getUint32(o + 28, true);
    const labelLen = dv.getUint16(o + 32, true);
    items[i] = {
      x: ox + dv.getFloat32(o, true) * size,
      y: oy + dv.getFloat32(o + 4, true) * size,
      w: dv.getFloat32(o + 8, true) * size,
      h: dv.getFloat32(o + 12, true) * size,
      slot: dv.getUint32(o + 16, true),
      kind: dv.getUint8(o + 20),
      depth: dv.getUint8(o + 21),
      flags: dv.getUint16(o + 22, true),
      hidden: dv.getUint32(o + 24, true),
      label: decoder.decode(buf.subarray(stringsAt + labelOff, stringsAt + labelOff + labelLen)),
      community: dv.getUint8(o + 34),
      owner: dv.getUint8(o + 35),
      load: dv.getUint16(o + 36, true),
      churn: dv.getUint16(o + 38, true),
    };
  }
  return { z, x: tx, y: ty, items };
}

export const tileKey = (z: number, x: number, y: number): string => `${z}/${x}/${y}`;

/** LRU cache of decoded tiles for the current epoch. */
export class TileCache {
  private readonly tiles = new Map<string, DecodedTile>();
  private readonly pending = new Set<string>();
  readonly capacity: number;
  constructor(capacity = 512) {
    this.capacity = capacity;
  }

  get(key: string): DecodedTile | undefined {
    const t = this.tiles.get(key);
    if (t) {
      // Refresh LRU position.
      this.tiles.delete(key);
      this.tiles.set(key, t);
    }
    return t;
  }

  has(key: string): boolean {
    return this.tiles.has(key);
  }

  isPending(key: string): boolean {
    return this.pending.has(key);
  }

  markPending(key: string): void {
    this.pending.add(key);
  }

  put(t: DecodedTile): void {
    const key = tileKey(t.z, t.x, t.y);
    this.pending.delete(key);
    this.tiles.set(key, t);
    while (this.tiles.size > this.capacity) {
      const oldest = this.tiles.keys().next().value as string;
      this.tiles.delete(oldest);
    }
  }

  failed(key: string): void {
    this.pending.delete(key);
  }

  clear(): void {
    this.tiles.clear();
    this.pending.clear();
  }
}
