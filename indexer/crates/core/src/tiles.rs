//! Map tiles. A tile is a square of the world at one zoom level; it carries every node that
//! intersects it and is big enough to draw at that zoom, packed as a little-endian binary buffer
//! so the webview never parses JSON for geometry.
//!
//! Format v1:
//!   header (24 B): magic u32 "TRRN", version u16, z u16, x u32, y u32, count u32, strings_len u32
//!   items  (40 B each):
//!     f32 x, y, w, h     position relative to the tile origin, in tile units
//!     u32 slot           node handle
//!     u8  kind, u8 depth, u16 flags   node flags; bits 14-15 are tile-only (Change lens)
//!     u32 hidden         files too small to draw at this zoom (aggregated into the parent)
//!     u32 label_off, u16 label_len   UTF-8 label in the string table
//!     u8  community, u8 owner, u16 load, u16 churn   lens values (0 = unknown / none)
//!   strings (strings_len B)

use crate::graph::Graph;
use crate::layout::{Layout, Rect};
use crate::model::*;

pub const MAGIC: u32 = u32::from_le_bytes(*b"TRRN");
pub const VERSION: u16 = 1;
pub const HEADER_BYTES: usize = 24;
pub const ITEM_BYTES: usize = 40;
/// Nominal tile size in pixels: at zoom z the world spans TILE_PX * 2^z pixels.
pub const TILE_PX: f64 = 512.0;
/// Smallest side, in pixels, at which a node is drawn.
pub const MIN_PX: f64 = 3.0;
/// File side, in pixels, from which its symbols are drawn (semantic zoom).
pub const SYMBOL_PX: f64 = 56.0;
pub const MAX_ZOOM: u32 = 22;
/// Items per tile. About 12–20 tiles cover a viewport, keeping the webview near 50k items.
pub const TILE_ITEM_LIMIT: usize = 4096;

/// Tile-only flag: changed on the current branch (Change lens).
pub const FLAG_CHANGED: u16 = 1 << 14;
/// Tile-only flag: depends on something changed on the current branch (blast radius).
pub const FLAG_BLAST: u16 = 1 << 15;

/// Per-node lens values packed into each item.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct LensValues {
    pub community: u8,
    pub owner: u8,
    pub load: u16,
    pub churn: u16,
    /// FLAG_CHANGED / FLAG_BLAST, or 0.
    pub change: u16,
}

#[derive(Clone, Debug)]
pub struct TileItem {
    pub slot: u32,
    pub rect: Rect,
    pub kind: NodeKind,
    pub depth: u8,
    pub flags: u16,
    pub hidden: u32,
}

struct Pending {
    area: f64,
    slot: u32,
    depth: u8,
    rect: Rect,
    symbol: bool,
}

impl PartialEq for Pending {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == std::cmp::Ordering::Equal
    }
}
impl Eq for Pending {}
impl PartialOrd for Pending {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Pending {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // Max-heap on area; ties broken by slot for determinism (smaller slot first).
        self.area
            .total_cmp(&o.area)
            .then_with(|| o.slot.cmp(&self.slot))
    }
}

pub fn tile_rect(z: u32, x: u32, y: u32) -> Rect {
    let size = 1.0 / (1u64 << z) as f64;
    Rect::new(x as f64 * size, y as f64 * size, size, size)
}

/// Collects drawable nodes for one tile. Work is proportional to the output: subtrees outside
/// the tile or below the size threshold are never visited.
pub fn query(
    g: &Graph,
    layout: &mut Layout,
    z: u32,
    x: u32,
    y: u32,
    limit: usize,
) -> Vec<TileItem> {
    let z = z.min(MAX_ZOOM);
    let bounds = tile_rect(z, x, y);
    let px_per_unit = TILE_PX * (1u64 << z) as f64;
    let mut out = Vec::new();
    // Largest first, so when `limit` cuts, the items that remain are the ones that matter, and
    // every container comes before its contents.
    let mut heap = std::collections::BinaryHeap::new();
    let root = layout.rects[Graph::ROOT as usize];
    heap.push(Pending {
        area: f64::INFINITY,
        slot: Graph::ROOT,
        depth: 0,
        rect: root,
        symbol: false,
    });
    while let Some(Pending {
        slot,
        depth,
        rect: r,
        symbol,
        ..
    }) = heap.pop()
    {
        if out.len() >= limit {
            break;
        }
        let node = g.node(slot);
        if symbol {
            out.push(TileItem {
                slot,
                rect: r,
                kind: node.kind,
                depth,
                flags: node.flags,
                hidden: 0,
            });
            continue;
        }
        if r.is_empty() || !r.intersects(&bounds) {
            continue;
        }
        if r.w * px_per_unit < MIN_PX || r.h * px_per_unit < MIN_PX {
            continue;
        }
        match node.kind {
            NodeKind::Dir => {
                let collapsed = layout.is_collapsed(g, slot);
                let mut hidden = 0u32;
                for &c in g.children(slot) {
                    let cr = layout.rects[c as usize];
                    if cr.w * px_per_unit < MIN_PX || cr.h * px_per_unit < MIN_PX {
                        hidden += layout.file_counts[c as usize];
                    } else if cr.intersects(&bounds) {
                        let d = if collapsed {
                            depth
                        } else {
                            depth.saturating_add(1)
                        };
                        // Collapsed dirs share their child's rect: keep the parent first.
                        let area = if collapsed {
                            f64::INFINITY
                        } else {
                            cr.w * cr.h
                        };
                        heap.push(Pending {
                            area,
                            slot: c,
                            depth: d,
                            rect: cr,
                            symbol: false,
                        });
                    }
                }
                if !collapsed && slot != Graph::ROOT {
                    out.push(TileItem {
                        slot,
                        rect: r,
                        kind: node.kind,
                        depth,
                        flags: node.flags,
                        hidden,
                    });
                }
            }
            NodeKind::File => {
                out.push(TileItem {
                    slot,
                    rect: r,
                    kind: node.kind,
                    depth,
                    flags: node.flags,
                    hidden: 0,
                });
                if r.w * px_per_unit >= SYMBOL_PX && r.h * px_per_unit >= SYMBOL_PX {
                    for &(s, sr) in layout.symbols(g, slot) {
                        if !sr.intersects(&bounds)
                            || sr.w * px_per_unit < MIN_PX
                            || sr.h * px_per_unit < MIN_PX
                        {
                            continue;
                        }
                        let mut d = depth.saturating_add(1);
                        let mut p = g.node(s).parent;
                        while p != slot && p != NONE {
                            d = d.saturating_add(1);
                            p = g.node(p).parent;
                        }
                        heap.push(Pending {
                            area: sr.w * sr.h,
                            slot: s,
                            depth: d,
                            rect: sr,
                            symbol: true,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Packs tile items into the binary format.
pub fn pack(
    g: &Graph,
    layout: &Layout,
    z: u32,
    x: u32,
    y: u32,
    items: &[TileItem],
    lens: impl Fn(u32) -> LensValues,
) -> Vec<u8> {
    let bounds = tile_rect(z, x, y);
    let inv = 1.0 / bounds.w;
    let mut strings = Vec::<u8>::new();
    let mut buf = Vec::with_capacity(HEADER_BYTES + items.len() * ITEM_BYTES);
    buf.extend_from_slice(&MAGIC.to_le_bytes());
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&(z as u16).to_le_bytes());
    buf.extend_from_slice(&x.to_le_bytes());
    buf.extend_from_slice(&y.to_le_bytes());
    buf.extend_from_slice(&(items.len() as u32).to_le_bytes());
    let strings_len_at = buf.len();
    buf.extend_from_slice(&0u32.to_le_bytes());
    for it in items {
        let r = it.rect;
        for v in [
            (r.x - bounds.x) * inv,
            (r.y - bounds.y) * inv,
            r.w * inv,
            r.h * inv,
        ] {
            buf.extend_from_slice(&(v as f32).to_le_bytes());
        }
        buf.extend_from_slice(&it.slot.to_le_bytes());
        buf.push(it.kind as u8);
        buf.push(it.depth);
        let l = lens(it.slot);
        let flags =
            (it.flags & !(FLAG_CHANGED | FLAG_BLAST)) | (l.change & (FLAG_CHANGED | FLAG_BLAST));
        buf.extend_from_slice(&flags.to_le_bytes());
        buf.extend_from_slice(&it.hidden.to_le_bytes());
        let label = layout.label(g, it.slot);
        let label = &label.as_bytes()[..label.len().min(u16::MAX as usize)];
        buf.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(label.len() as u16).to_le_bytes());
        strings.extend_from_slice(label);
        buf.push(l.community);
        buf.push(l.owner);
        buf.extend_from_slice(&l.load.to_le_bytes());
        buf.extend_from_slice(&l.churn.to_le_bytes());
    }
    let sl = strings.len() as u32;
    buf[strings_len_at..strings_len_at + 4].copy_from_slice(&sl.to_le_bytes());
    buf.extend_from_slice(&strings);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::SymbolSpec;

    fn graph() -> Graph {
        let mut g = Graph::new("repo");
        for i in 0..200 {
            g.upsert_file(&format!("pkg{}/f{i}.rs", i % 7), 10 + i, Lang::Rust, 0);
        }
        g.upsert_file("big.rs", 5000, Lang::Rust, 0);
        g
    }

    #[test]
    fn zooming_in_reveals_more_files_and_counts_hidden_ones() {
        let g = graph();
        let mut l = Layout::compute(&g);
        let z0 = query(&g, &mut l, 0, 0, 0, 100_000);
        let files0 = z0.iter().filter(|i| i.kind == NodeKind::File).count();
        let hidden0: u32 = z0.iter().map(|i| i.hidden).sum();
        assert_eq!(
            files0 as u32 + hidden0,
            201,
            "every file is either drawn or counted as hidden"
        );
        let mut files3 = 0;
        for x in 0..8 {
            for y in 0..8 {
                files3 += query(&g, &mut l, 3, x, y, 100_000)
                    .iter()
                    .filter(|i| i.kind == NodeKind::File)
                    .count();
            }
        }
        assert!(files3 >= files0);
    }

    #[test]
    fn symbols_appear_only_when_the_file_is_large_on_screen() {
        let mut g = graph();
        let big = g.slot_of_path("big.rs").unwrap();
        g.replace_symbols(
            big,
            &[SymbolSpec {
                kind: NodeKind::Function,
                name: "main".into(),
                parent: None,
                start_line: 1,
                end_line: 2000,
                flags: 0,
            }],
        );
        let mut l = Layout::compute(&g);
        let has_sym = |items: &[TileItem]| items.iter().any(|i| i.kind == NodeKind::Function);
        let r = l.rects[big as usize];
        assert!(!has_sym(&query(&g, &mut l, 0, 0, 0, 100_000)) || r.w * TILE_PX >= SYMBOL_PX);
        let z = 4;
        let n = 1u32 << z;
        let mut found = false;
        for tx in (r.x * n as f64) as u32..=((r.x + r.w) * n as f64) as u32 {
            for ty in (r.y * n as f64) as u32..=((r.y + r.h) * n as f64) as u32 {
                found |= has_sym(&query(&g, &mut l, z, tx, ty, 100_000));
            }
        }
        assert!(found);
    }

    #[test]
    fn limit_keeps_the_largest_items_and_parents_first() {
        let g = graph();
        let mut l = Layout::compute(&g);
        let all = query(&g, &mut l, 0, 0, 0, 100_000);
        let some = query(&g, &mut l, 0, 0, 0, 20);
        assert_eq!(some.len(), 20);
        let min_kept = some
            .iter()
            .map(|i| i.rect.w * i.rect.h)
            .fold(f64::INFINITY, f64::min);
        let bigger_dropped = all
            .iter()
            .filter(|i| !some.iter().any(|k| k.slot == i.slot))
            .any(|i| i.rect.w * i.rect.h > min_kept + 1e-15);
        assert!(!bigger_dropped);
        for (i, it) in all.iter().enumerate() {
            let parent = g.node(it.slot).parent;
            if let Some(pi) = all.iter().position(|o| o.slot == parent) {
                assert!(pi < i, "parent after child");
            }
        }
    }

    #[test]
    fn pack_writes_header_items_and_labels() {
        let g = graph();
        let mut l = Layout::compute(&g);
        let items = query(&g, &mut l, 0, 0, 0, 100_000);
        let buf = pack(&g, &l, 0, 0, 0, &items, |_| LensValues {
            community: 3,
            ..Default::default()
        });
        assert_eq!(u32::from_le_bytes(buf[0..4].try_into().unwrap()), MAGIC);
        let count = u32::from_le_bytes(buf[16..20].try_into().unwrap()) as usize;
        let slen = u32::from_le_bytes(buf[20..24].try_into().unwrap()) as usize;
        assert_eq!(count, items.len());
        assert_eq!(buf.len(), HEADER_BYTES + count * ITEM_BYTES + slen);
        assert_eq!(buf[HEADER_BYTES + 34], 3, "community byte");
    }
}
