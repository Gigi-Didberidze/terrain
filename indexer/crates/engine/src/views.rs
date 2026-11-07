//! Queries behind the Neighbourhood and Path views.
//!
//! Neighbourhood: the node in the centre, what depends on it in columns to the left (callers,
//! importers) and what it depends on to the right, two levels each, at the centre's
//! granularity (symbols around a symbol, files around a file). Every edge drawn points left to
//! right, so the view reads like a sentence: "A uses B uses C".
//!
//! Path: up to three shortest dependency chains between two nodes (Yen's algorithm over
//! unweighted BFS), at symbol level when both ends are symbols, else between their files.

use crate::{EdgeView, Engine};
use std::collections::{HashMap, HashSet, VecDeque};
use terrain_core::model::*;

/// Per column, at most this many nodes (strongest edges first); the rest are counted.
pub const COLUMN_LIMIT: usize = 16;
/// Per node in the first column, at most this many of its own neighbours in the second.
pub const FANOUT_LIMIT: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct NbNode {
    pub slot: u32,
    /// -2, -1 (depend on the centre), 0 (centre), 1, 2 (the centre depends on them).
    pub level: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NbEdge {
    /// The dependent: `from` uses `to`.
    pub from: u32,
    pub to: u32,
    pub kind: EdgeKind,
    pub conf: Confidence,
    pub count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Neighbourhood {
    pub nodes: Vec<NbNode>,
    pub edges: Vec<NbEdge>,
    /// Direct dependents and dependencies in total (columns -1 and 1 may show fewer).
    pub dependents: u32,
    pub dependencies: u32,
    pub millis: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Paths {
    /// Each path from `from` to `to`, both included, shortest first.
    pub paths: Vec<Vec<u32>>,
    /// True when no chain leads from `from` to `to` and the paths run the other way.
    pub reversed: bool,
    /// The ends actually used (files when one end was not a symbol).
    pub from: u32,
    pub to: u32,
}

impl Engine {
    pub fn neighbourhood(&self, slot: u32) -> Neighbourhood {
        let t0 = std::time::Instant::now();
        let mut out = Neighbourhood::default();
        if self.graph.get(slot).is_none() {
            return out;
        }
        let mut cache: HashMap<u32, Vec<EdgeView>> = HashMap::new();
        let mut edges_of = |s: u32| -> Vec<EdgeView> {
            cache
                .entry(s)
                .or_insert_with(|| self.edges_of(s, usize::MAX))
                .clone()
        };
        let centre_edges = edges_of(slot);
        out.dependents = centre_edges.iter().filter(|e| !e.outgoing).count() as u32;
        out.dependencies = centre_edges.iter().filter(|e| e.outgoing).count() as u32;
        let mut level_of: HashMap<u32, i32> = HashMap::from([(slot, 0)]);
        out.nodes.push(NbNode { slot, level: 0 });
        for side in [-1i32, 1] {
            let outgoing = side > 0;
            let rank = |v: &mut Vec<EdgeView>| {
                v.sort_by(|a, b| {
                    b.weight
                        .total_cmp(&a.weight)
                        .then(
                            self.analysis
                                .rank_of(b.other)
                                .total_cmp(&self.analysis.rank_of(a.other)),
                        )
                        .then(a.other.cmp(&b.other))
                })
            };
            let mut first: Vec<EdgeView> = centre_edges
                .iter()
                .filter(|e| e.outgoing == outgoing)
                .cloned()
                .collect();
            rank(&mut first);
            let mut column1 = Vec::new();
            for e in first {
                if column1.len() == COLUMN_LIMIT {
                    break;
                }
                if level_of.contains_key(&e.other) {
                    continue;
                }
                level_of.insert(e.other, side);
                out.nodes.push(NbNode {
                    slot: e.other,
                    level: side,
                });
                column1.push(e.other);
            }
            let mut column2 = 0;
            'outer: for &n in &column1 {
                let mut next: Vec<EdgeView> = edges_of(n)
                    .into_iter()
                    .filter(|e| e.outgoing == outgoing)
                    .collect();
                rank(&mut next);
                for e in next.into_iter().take(FANOUT_LIMIT) {
                    if column2 == COLUMN_LIMIT {
                        break 'outer;
                    }
                    if level_of.contains_key(&e.other) {
                        continue;
                    }
                    level_of.insert(e.other, 2 * side);
                    out.nodes.push(NbNode {
                        slot: e.other,
                        level: 2 * side,
                    });
                    column2 += 1;
                }
            }
        }
        // Edges among the shown nodes, left to right only (a column further right is used by one
        // further left); same-column and backward edges belong to cycles and are left out.
        let shown: Vec<u32> = out.nodes.iter().map(|n| n.slot).collect();
        // Raw edges from everything inside each shown node; a target inside a shown node (a
        // method of a shown class, a symbol of a shown file) attaches to the nearest one.
        let nearest_shown = |mut t: u32| -> Option<u32> {
            loop {
                if level_of.contains_key(&t) {
                    return Some(t);
                }
                let n = self.graph.node(t);
                if n.kind == NodeKind::Dir || n.parent == NONE {
                    return None;
                }
                t = n.parent;
            }
        };
        for &s in &shown {
            let ls = level_of[&s];
            for inner in self.subtree(s, 50_000) {
                // A shown node inside this one draws its own edges.
                if inner != s && level_of.contains_key(&inner) {
                    continue;
                }
                for e in self.graph.out_edges(inner) {
                    if !e.kind.is_dependency() {
                        continue;
                    }
                    let Some(target) = nearest_shown(e.other) else {
                        continue;
                    };
                    if target == s {
                        continue;
                    }
                    let lt = level_of[&target];
                    // Edges jumping across the centre (a caller's caller using a callee) are
                    // true but tangential, and would cross the whole diagram.
                    let crosses = ls < 0 && lt > 0;
                    if lt > ls && !crosses {
                        out.edges.push(NbEdge {
                            from: s,
                            to: target,
                            kind: e.kind,
                            conf: e.conf,
                            count: 1,
                        });
                    }
                }
            }
        }
        // Several inner targets can map to one shown node: merge them.
        out.edges.sort_by_key(|e| (e.from, e.to));
        out.edges.dedup_by(|b, a| {
            if a.from == b.from && a.to == b.to {
                a.count += b.count;
                if b.conf == Confidence::Semantic {
                    a.conf = Confidence::Semantic;
                }
                true
            } else {
                false
            }
        });
        out.millis = t0.elapsed().as_secs_f64() * 1000.0;
        out
    }

    /// Up to `k` shortest dependency chains from `from` to `to` (or, failing that, from `to` to
    /// `from`, with `reversed` set).
    pub fn paths(&self, from: u32, to: u32, k: usize) -> Paths {
        let (Some(a), Some(b)) = (self.graph.get(from), self.graph.get(to)) else {
            return Paths::default();
        };
        let symbols = a.kind.is_symbol() && b.kind.is_symbol();
        let lift = |s: u32| {
            let n = self.graph.node(s);
            if n.kind.is_symbol() {
                n.file
            } else {
                s
            }
        };
        let (from, to) = if symbols {
            (from, to)
        } else {
            (lift(from), lift(to))
        };
        let next = |s: u32| -> Vec<u32> {
            if symbols {
                let mut v: Vec<u32> = self
                    .graph
                    .out_edges(s)
                    .iter()
                    .filter(|e| e.kind.is_dependency() && self.graph.node(e.other).kind.is_symbol())
                    .map(|e| e.other)
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            } else {
                let a = &self.analysis;
                match a.file_index(s) {
                    Some(i) => a
                        .deps
                        .neighbors(i)
                        .map(|(t, _)| a.files[t as usize])
                        .collect(),
                    None => Vec::new(),
                }
            }
        };
        let forward = k_shortest(from, to, k, &next);
        if !forward.is_empty() || from == to {
            return Paths {
                paths: forward,
                reversed: false,
                from,
                to,
            };
        }
        let back = k_shortest(to, from, k, &next);
        Paths {
            reversed: !back.is_empty(),
            paths: back,
            from,
            to,
        }
    }
}

/// Shortest path by BFS avoiding removed nodes and edges; neighbours visited in the order
/// `next` gives them, so results are deterministic.
fn bfs(
    from: u32,
    to: u32,
    next: &dyn Fn(u32) -> Vec<u32>,
    banned_nodes: &HashSet<u32>,
    banned_edges: &HashSet<(u32, u32)>,
) -> Option<Vec<u32>> {
    let mut prev: HashMap<u32, u32> = HashMap::from([(from, from)]);
    let mut q = VecDeque::from([from]);
    // Bounded so a query on a huge graph stays interactive.
    let mut budget = 200_000usize;
    while let Some(s) = q.pop_front() {
        if s == to {
            let mut path = vec![to];
            let mut cur = to;
            while cur != from {
                cur = prev[&cur];
                path.push(cur);
            }
            path.reverse();
            return Some(path);
        }
        for t in next(s) {
            budget = budget.checked_sub(1)?;
            if prev.contains_key(&t) || banned_nodes.contains(&t) || banned_edges.contains(&(s, t))
            {
                continue;
            }
            prev.insert(t, s);
            q.push_back(t);
        }
    }
    None
}

/// Yen's k shortest loopless paths (unweighted).
fn k_shortest(from: u32, to: u32, k: usize, next: &dyn Fn(u32) -> Vec<u32>) -> Vec<Vec<u32>> {
    let none = HashSet::new();
    let Some(first) = bfs(from, to, next, &none, &HashSet::new()) else {
        return Vec::new();
    };
    let mut found = vec![first];
    let mut candidates: Vec<Vec<u32>> = Vec::new();
    while found.len() < k {
        let last = found.last().unwrap().clone();
        for i in 0..last.len().saturating_sub(1) {
            let spur = last[i];
            let root = &last[..=i];
            let mut banned_edges = HashSet::new();
            for p in &found {
                if p.len() > i && p[..=i] == *root {
                    banned_edges.insert((p[i], p[i + 1]));
                }
            }
            let banned_nodes: HashSet<u32> = root[..i].iter().copied().collect();
            if let Some(tail) = bfs(spur, to, next, &banned_nodes, &banned_edges) {
                let mut p = root[..i].to_vec();
                p.extend(tail);
                if !found.contains(&p) && !candidates.contains(&p) {
                    candidates.push(p);
                }
            }
        }
        if candidates.is_empty() {
            break;
        }
        // Shortest candidate; ties by content for determinism.
        candidates.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        found.push(candidates.remove(0));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{Engine, Options};
    use terrain_core::graph::SymbolSpec;

    fn spec(kind: NodeKind, name: &str, parent: Option<usize>) -> SymbolSpec {
        SymbolSpec {
            kind,
            name: name.into(),
            parent,
            start_line: 1,
            end_line: 1,
            flags: 0,
        }
    }

    /// caller() → Widget.render() → centre() → helper() → leaf(); plus a test file at
    /// top level calling centre().
    fn engine() -> (Engine, HashMap<&'static str, u32>) {
        let mut e = Engine::new(std::path::Path::new("ws"), None, Options::default());
        let g = &mut e.graph;
        let mut slots = HashMap::new();
        let f = g.upsert_file("a.ts", 10, Lang::TypeScript, 0);
        let d = g.replace_symbols(
            f,
            &[
                spec(NodeKind::Function, "caller", None),
                spec(NodeKind::Class, "Widget", None),
                spec(NodeKind::Method, "render", Some(1)),
                spec(NodeKind::Function, "centre", None),
                spec(NodeKind::Function, "helper", None),
                spec(NodeKind::Function, "leaf", None),
            ],
        );
        for (i, n) in ["caller", "Widget", "render", "centre", "helper", "leaf"]
            .into_iter()
            .enumerate()
        {
            slots.insert(n, d.slots[i]);
        }
        let t = g.upsert_file("a.test.ts", 5, Lang::TypeScript, 0);
        slots.insert("test", t);
        let c = |g: &mut terrain_core::Graph, a: u32, b: u32| {
            g.add_edge(a, b, EdgeKind::Calls, Confidence::Syntactic, 1.0)
        };
        c(g, slots["caller"], slots["render"]);
        c(g, slots["render"], slots["centre"]);
        c(g, slots["centre"], slots["helper"]);
        c(g, slots["helper"], slots["leaf"]);
        c(g, t, slots["centre"]);
        (e, slots)
    }

    #[test]
    fn neighbourhood_columns_and_edges_read_left_to_right() {
        let (e, s) = engine();
        let nb = e.neighbourhood(s["centre"]);
        let level = |n: &str| nb.nodes.iter().find(|x| x.slot == s[n]).map(|x| x.level);
        assert_eq!(level("centre"), Some(0));
        // Around a symbol, dependents are symbols (the method itself) or files (top-level code).
        assert_eq!(level("render"), Some(-1));
        assert_eq!(level("test"), Some(-1));
        assert_eq!(level("Widget"), None);
        assert_eq!(level("caller"), Some(-2));
        assert_eq!(level("helper"), Some(1));
        assert_eq!(level("leaf"), Some(2));
        let has = |a: &str, b: &str| nb.edges.iter().any(|x| x.from == s[a] && x.to == s[b]);
        assert!(has("caller", "render") && has("render", "centre") && has("test", "centre"));
        assert!(has("centre", "helper") && has("helper", "leaf"));
        assert_eq!(nb.edges.len(), 5);
        assert_eq!((nb.dependents, nb.dependencies), (2, 1));
    }

    #[test]
    fn edges_attach_to_the_nearest_shown_container() {
        // Around a file, the class is what shows; caller() reaches it through render().
        let (mut e, s) = engine();
        let b = e.graph.upsert_file("b.ts", 3, Lang::TypeScript, 0);
        e.graph.add_edge(
            s["caller"],
            b,
            EdgeKind::Imports,
            Confidence::Syntactic,
            1.0,
        );
        let nb = e.neighbourhood(b);
        assert_eq!(
            nb.nodes
                .iter()
                .find(|x| x.slot == s["caller"])
                .map(|x| x.level),
            None,
            "caller is a symbol in a.ts"
        );
        let a = e.graph.slot_of_path("a.ts").unwrap();
        assert_eq!(
            nb.nodes.iter().find(|x| x.slot == a).map(|x| x.level),
            Some(-1)
        );
        assert!(nb.edges.iter().any(|x| x.from == a && x.to == b));
    }

    #[test]
    fn paths_between_symbols_and_reversed() {
        let (e, s) = engine();
        let p = e.paths(s["caller"], s["leaf"], 3);
        assert_eq!(
            p.paths,
            vec![vec![
                s["caller"],
                s["render"],
                s["centre"],
                s["helper"],
                s["leaf"]
            ]]
        );
        assert!(!p.reversed);
        let back = e.paths(s["leaf"], s["caller"], 3);
        assert!(back.reversed);
        assert_eq!(back.paths.len(), 1);
    }

    #[test]
    fn yen_finds_alternatives_shortest_first() {
        // 1 → 2 → 4, 1 → 3 → 4, 1 → 5 → 6 → 4
        let g: HashMap<u32, Vec<u32>> = HashMap::from([
            (1, vec![2, 3, 5]),
            (2, vec![4]),
            (3, vec![4]),
            (5, vec![6]),
            (6, vec![4]),
        ]);
        let next = |s: u32| g.get(&s).cloned().unwrap_or_default();
        let p = k_shortest(1, 4, 3, &next);
        assert_eq!(p, vec![vec![1, 2, 4], vec![1, 3, 4], vec![1, 5, 6, 4]]);
        assert!(k_shortest(4, 1, 3, &next).is_empty());
        assert_eq!(k_shortest(1, 4, 1, &next).len(), 1);
    }
}
