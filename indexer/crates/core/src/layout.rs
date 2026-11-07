//! Deterministic squarified treemap. Position comes from the directory tree, area from lines of
//! code. The layout depends only on the tree and LOC, never on the window, so people learn where
//! things are. Symbols are laid out lazily inside their file when someone zooms that far.

use crate::graph::Graph;
use crate::model::*;
use std::collections::HashMap;

/// The world is the unit square; the map occupies the top WORLD_H of it (16:10).
pub const WORLD_W: f64 = 1.0;
pub const WORLD_H: f64 = 0.625;
/// Header band for labels, as a fraction of a container's height.
pub const HEADER: f64 = 0.055;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    /// Inner area for children: a thin frame plus a header band for the label.
    pub fn inner(&self, header: bool) -> Rect {
        let pad = self.w.min(self.h) * 0.008;
        let head = if header {
            (self.h * HEADER).min(self.w * 0.15)
        } else {
            0.0
        };
        Rect::new(
            self.x + pad,
            self.y + pad + head,
            (self.w - 2.0 * pad).max(0.0),
            (self.h - 2.0 * pad - head).max(0.0),
        )
    }
}

pub struct Layout {
    /// Indexed by slot; empty rect when the node has no place (dead, symbol not laid out).
    pub rects: Vec<Rect>,
    /// Files in the subtree (for "+ 312 small files" aggregation labels).
    pub file_counts: Vec<u32>,
    /// Weight (LOC) of each dir/file subtree.
    pub weights: Vec<f64>,
    /// Display label of collapsed chains: shown on the deepest dir, e.g. "src/vs/base".
    pub labels: HashMap<u32, String>,
    pub epoch: u64,
    symbol_cache: HashMap<u32, Vec<(u32, Rect)>>,
    symbol_epochs: HashMap<u32, u64>,
}

impl Layout {
    pub fn compute(g: &Graph) -> Layout {
        let n = g.nodes.len();
        let mut l = Layout {
            rects: vec![Rect::default(); n],
            file_counts: vec![0; n],
            weights: vec![0.0; n],
            labels: HashMap::new(),
            epoch: g.layout_epoch,
            symbol_cache: HashMap::new(),
            symbol_epochs: HashMap::new(),
        };
        l.weigh(g, Graph::ROOT);
        let root = Rect::new(0.0, 0.0, WORLD_W, WORLD_H);
        l.rects[Graph::ROOT as usize] = root;
        l.place_children(g, Graph::ROOT, root, String::new());
        l
    }

    fn weigh(&mut self, g: &Graph, slot: u32) -> (f64, u32) {
        let node = g.node(slot);
        let (w, c) = match node.kind {
            NodeKind::File => (node.loc.max(1) as f64, 1),
            NodeKind::Dir => {
                let mut w = 0.0;
                let mut c = 0;
                for &ch in g.children(slot) {
                    let (cw, cc) = self.weigh(g, ch);
                    w += cw;
                    c += cc;
                }
                (w, c)
            }
            _ => (0.0, 0),
        };
        self.weights[slot as usize] = w;
        self.file_counts[slot as usize] = c;
        (w, c)
    }

    /// Lays out the children of `dir` inside `rect`. `chain` carries the collapsed prefix.
    fn place_children(&mut self, g: &Graph, dir: u32, rect: Rect, chain: String) {
        let kids: Vec<u32> = g
            .children(dir)
            .iter()
            .copied()
            .filter(|&c| g.node(c).kind != NodeKind::Dir || self.weights[c as usize] > 0.0)
            .collect();
        // Collapse single-child directory chains: the child takes the whole rect.
        if dir != Graph::ROOT && kids.len() == 1 && g.node(kids[0]).kind == NodeKind::Dir {
            let child = kids[0];
            self.rects[child as usize] = rect;
            let label = format!("{}{}/", chain, g.node(dir).name);
            self.place_children(g, child, rect, label);
            return;
        }
        if dir != Graph::ROOT && !chain.is_empty() {
            self.labels
                .insert(dir, format!("{}{}", chain, g.node(dir).name));
        }
        let inner = if dir == Graph::ROOT {
            rect
        } else {
            rect.inner(true)
        };
        let mut items: Vec<(u32, f64)> = kids
            .iter()
            .map(|&c| (c, self.weights[c as usize]))
            .collect();
        items.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| g.node(a.0).name.cmp(&g.node(b.0).name))
        });
        let mut out = Vec::with_capacity(items.len());
        squarify(&items, inner, &mut out);
        for (slot, r) in out {
            self.rects[slot as usize] = r;
            if g.node(slot).kind == NodeKind::Dir {
                self.place_children(g, slot, r, String::new());
            }
        }
    }

    /// Whether `dir` is drawn merged into its only child.
    pub fn is_collapsed(&self, g: &Graph, dir: u32) -> bool {
        let n = g.node(dir);
        if n.kind != NodeKind::Dir || dir == Graph::ROOT {
            return false;
        }
        let kids = g.children(dir);
        kids.len() == 1 && g.node(kids[0]).kind == NodeKind::Dir
    }

    pub fn label<'a>(&'a self, g: &'a Graph, slot: u32) -> &'a str {
        self.labels
            .get(&slot)
            .map(|s| s.as_str())
            .unwrap_or(&g.node(slot).name)
    }

    /// Symbol rectangles inside a file, laid out on demand and cached until the graph changes.
    pub fn symbols(&mut self, g: &Graph, file: u32) -> &[(u32, Rect)] {
        let fresh = self.symbol_epochs.get(&file) == Some(&g.epoch);
        if !fresh {
            let mut out = Vec::new();
            let r = self.rects[file as usize];
            if !r.is_empty() {
                let file_lines = g.node(file).loc.max(1) as f64;
                place_symbols(g, file, file_lines, r.inner(true), &mut out);
            }
            self.symbol_cache.insert(file, out);
            self.symbol_epochs.insert(file, g.epoch);
        }
        &self.symbol_cache[&file]
    }

    /// Rect for any node, laying out symbols if needed.
    pub fn rect_of(&mut self, g: &Graph, slot: u32) -> Option<Rect> {
        let n = g.get(slot)?;
        if n.kind.is_symbol() {
            let file = n.file;
            return self
                .symbols(g, file)
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, r)| *r);
        }
        let r = *self.rects.get(slot as usize)?;
        (!r.is_empty()).then_some(r)
    }
}

/// Lays out the symbol children of `parent` in `rect`; area is proportional to line span, and
/// the lines not covered by any child remain as empty space so area keeps meaning LOC.
fn place_symbols(
    g: &Graph,
    parent: u32,
    parent_lines: f64,
    rect: Rect,
    out: &mut Vec<(u32, Rect)>,
) {
    let kids = g.children(parent);
    if kids.is_empty() || rect.is_empty() {
        return;
    }
    let mut items: Vec<(u32, f64)> = kids.iter().map(|&c| (c, symbol_weight(g, c))).collect();
    let covered: f64 = items.iter().map(|i| i.1).sum();
    let rest = (parent_lines - covered).max(0.0);
    items.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| g.node(a.0).start_line.cmp(&g.node(b.0).start_line))
    });
    if rest > 0.0 {
        items.push((NONE, rest));
    }
    let mut placed = Vec::with_capacity(items.len());
    squarify(&items, rect, &mut placed);
    for (slot, r) in placed {
        if slot == NONE {
            continue;
        }
        out.push((slot, r));
        if !g.children(slot).is_empty() {
            place_symbols(g, slot, symbol_weight(g, slot), r.inner(true), out);
        }
    }
}

/// Lines a symbol occupies on the map: its own span, or the sum of its children when they live
/// outside it (Go methods attached to their receiver type), whichever is larger.
fn symbol_weight(g: &Graph, slot: u32) -> f64 {
    let own = g.node(slot).loc.max(1) as f64;
    let kids: f64 = g.children(slot).iter().map(|&c| symbol_weight(g, c)).sum();
    own.max(kids)
}

/// Squarified treemap (Bruls, Huizing, van Wijk 2000). Items must be sorted by weight desc.
pub fn squarify(items: &[(u32, f64)], rect: Rect, out: &mut Vec<(u32, Rect)>) {
    let total: f64 = items.iter().map(|i| i.1).sum();
    if items.is_empty() || total <= 0.0 || rect.is_empty() {
        for &(s, _) in items {
            out.push((s, Rect::new(rect.x, rect.y, 0.0, 0.0)));
        }
        return;
    }
    let scale = rect.w * rect.h / total;
    let areas: Vec<f64> = items.iter().map(|i| i.1 * scale).collect();
    let mut r = rect;
    let mut i = 0;
    while i < items.len() {
        let short = r.w.min(r.h);
        let mut j = i + 1;
        let mut sum = areas[i];
        let mut worst = worst_ratio(areas[i], areas[i], sum, short);
        while j < items.len() {
            let nsum = sum + areas[j];
            // Rows are sorted desc, so the row max is areas[i] and the min is areas[j].
            let w = worst_ratio(areas[i], areas[j], nsum, short);
            if w > worst {
                break;
            }
            worst = w;
            sum = nsum;
            j += 1;
        }
        let last_row = j == items.len();
        if r.w >= r.h {
            // Column along the left edge.
            let thick = if last_row {
                r.w
            } else if r.h > 0.0 {
                sum / r.h
            } else {
                0.0
            };
            let mut y = r.y;
            for k in i..j {
                let h = if sum > 0.0 { areas[k] / sum * r.h } else { 0.0 };
                let h = if k + 1 == j { r.y + r.h - y } else { h };
                out.push((items[k].0, Rect::new(r.x, y, thick, h)));
                y += h;
            }
            r.x += thick;
            r.w = (r.w - thick).max(0.0);
        } else {
            let thick = if last_row {
                r.h
            } else if r.w > 0.0 {
                sum / r.w
            } else {
                0.0
            };
            let mut x = r.x;
            for k in i..j {
                let w = if sum > 0.0 { areas[k] / sum * r.w } else { 0.0 };
                let w = if k + 1 == j { r.x + r.w - x } else { w };
                out.push((items[k].0, Rect::new(x, r.y, w, thick)));
                x += w;
            }
            r.y += thick;
            r.h = (r.h - thick).max(0.0);
        }
        i = j;
    }
}

fn worst_ratio(max: f64, min: f64, sum: f64, short: f64) -> f64 {
    if min <= 0.0 || sum <= 0.0 || short <= 0.0 {
        return f64::INFINITY;
    }
    let s2 = sum * sum;
    let w2 = short * short;
    (w2 * max / s2).max(s2 / (w2 * min))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_preserves_area_and_stays_inside() {
        let items: Vec<(u32, f64)> = [60.0, 30.0, 20.0, 10.0, 5.0, 1.0]
            .iter()
            .enumerate()
            .map(|(i, &w)| (i as u32, w))
            .collect();
        let rect = Rect::new(0.1, 0.2, 0.6, 0.4);
        let mut out = Vec::new();
        squarify(&items, rect, &mut out);
        let total: f64 = items.iter().map(|i| i.1).sum();
        for ((_, w), (_, r)) in items.iter().zip(&out) {
            let expect = w / total * rect.w * rect.h;
            assert!((r.w * r.h - expect).abs() < 1e-9, "area mismatch");
            assert!(r.x >= rect.x - 1e-12 && r.y >= rect.y - 1e-12);
            assert!(r.x + r.w <= rect.x + rect.w + 1e-9 && r.y + r.h <= rect.y + rect.h + 1e-9);
        }
        // No overlaps.
        for a in 0..out.len() {
            for b in a + 1..out.len() {
                let (ra, rb) = (out[a].1, out[b].1);
                let ix = (ra.x + ra.w).min(rb.x + rb.w) - ra.x.max(rb.x);
                let iy = (ra.y + ra.h).min(rb.y + rb.h) - ra.y.max(rb.y);
                assert!(ix <= 1e-9 || iy <= 1e-9, "overlap {a} {b}");
            }
        }
    }

    fn sample() -> Graph {
        let mut g = Graph::new("repo");
        g.upsert_file("src/a.rs", 100, Lang::Rust, 0);
        g.upsert_file("src/b.rs", 50, Lang::Rust, 0);
        g.upsert_file("deep/x/y/z.rs", 30, Lang::Rust, 0);
        g.upsert_file("README.md", 10, Lang::Markdown, 0);
        g
    }

    #[test]
    fn layout_is_deterministic_and_collapses_chains() {
        let g = sample();
        let a = Layout::compute(&g);
        let b = Layout::compute(&g);
        assert_eq!(a.rects, b.rects);
        let deep = g.slot_of_path("deep").unwrap();
        let y = g.slot_of_path("deep/x/y").unwrap();
        assert!(a.is_collapsed(&g, deep));
        assert_eq!(a.rects[deep as usize], a.rects[y as usize]);
        assert_eq!(a.label(&g, y), "deep/x/y");
    }

    #[test]
    fn adding_a_file_elsewhere_moves_little() {
        let mut g = sample();
        let before = Layout::compute(&g);
        g.upsert_file("README2.md", 1, Lang::Markdown, 0);
        let after = Layout::compute(&g);
        let a = g.slot_of_path("src/a.rs").unwrap() as usize;
        let (r0, r1) = (before.rects[a], after.rects[a]);
        assert!((r0.x - r1.x).abs() < 0.02 && (r0.y - r1.y).abs() < 0.02);
    }
}
