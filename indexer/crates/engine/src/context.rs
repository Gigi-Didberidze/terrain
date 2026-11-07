//! Per-node context for the sidebar and CodeLens: direct callers/callees, importers, the blast
//! radius (transitive dependents, depth ≤ 6), the tests that reach a node, and complexity.

use crate::Engine;
use compass_core::model::*;
use std::collections::{HashMap, HashSet, VecDeque};

/// Depth cap for blast radius (the plan's display limit).
pub const BLAST_DEPTH: u32 = 6;
/// Node visits per symbol-level blast walk; dense call graphs stay bounded.
const BLAST_VISIT_LIMIT: usize = 20_000;

#[derive(Clone, Debug)]
pub struct NodeRef {
    pub slot: u32,
    pub name: String,
    pub kind: NodeKind,
    pub path: String,
    pub line: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    pub callers: Vec<NodeRef>,
    pub callees: Vec<NodeRef>,
    pub importers: Vec<NodeRef>,
    pub imports: Vec<NodeRef>,
    pub tests: Vec<NodeRef>,
    /// Files that transitively depend on the node, most load-bearing first (capped).
    pub blast: Vec<NodeRef>,
    pub blast_count: u32,
}

#[derive(Clone, Debug)]
pub struct Lens {
    pub slot: u32,
    pub line: u32,
    pub callers: u32,
    pub blast: u32,
    pub tests: u32,
    pub complexity: u32,
}

impl Engine {
    pub fn node_ref(&self, slot: u32) -> NodeRef {
        let n = self.graph.node(slot);
        let file = if n.kind.is_symbol() { n.file } else { slot };
        NodeRef {
            slot,
            name: if n.kind.is_symbol() {
                self.graph.qualified_name(slot)
            } else {
                n.name.to_string()
            },
            kind: n.kind,
            path: self.graph.path(file).unwrap_or("").to_string(),
            line: if n.kind.is_symbol() { n.start_line } else { 1 },
        }
    }

    fn refs(&self, mut slots: Vec<u32>, limit: usize) -> Vec<NodeRef> {
        slots.sort_unstable();
        slots.dedup();
        // Most load-bearing first.
        slots.sort_by(|a, b| {
            self.analysis
                .rank_of(*b)
                .total_cmp(&self.analysis.rank_of(*a))
                .then(a.cmp(b))
        });
        slots
            .into_iter()
            .take(limit)
            .map(|s| self.node_ref(s))
            .collect()
    }

    /// Files that transitively depend on `slot` (within BLAST_DEPTH), excluding its own file.
    pub fn blast_files(&self, slot: u32) -> Vec<u32> {
        let Some(n) = self.graph.get(slot) else {
            return Vec::new();
        };
        if n.kind == NodeKind::File {
            return self
                .analysis
                .file_blast(slot, BLAST_DEPTH, usize::MAX)
                .into_iter()
                .map(|(f, _)| f)
                .collect();
        }
        if n.kind == NodeKind::Dir {
            // Everything outside the directory that depends on anything inside it.
            let inside: HashSet<u32> = self
                .graph
                .files()
                .filter(|(_, p)| {
                    self.graph
                        .path(slot)
                        .is_some_and(|d| d.is_empty() || p.starts_with(&format!("{d}/")))
                })
                .map(|(f, _)| f)
                .collect();
            let mut out = HashSet::new();
            for &f in &inside {
                for (d, _) in self.analysis.file_blast(f, BLAST_DEPTH, usize::MAX) {
                    if !inside.contains(&d) {
                        out.insert(d);
                    }
                }
            }
            return out.into_iter().collect();
        }
        // Symbols: walk dependents at symbol level (more precise than the file graph).
        let own = n.file;
        let mut seen: HashSet<u32> = HashSet::from([slot]);
        let mut files: HashSet<u32> = HashSet::new();
        let mut queue: VecDeque<(u32, u32)> = VecDeque::from([(slot, 0)]);
        while let Some((v, depth)) = queue.pop_front() {
            if depth >= BLAST_DEPTH || seen.len() > BLAST_VISIT_LIMIT {
                continue;
            }
            for e in self.graph.in_edges(v) {
                if !e.kind.is_dependency() || !seen.insert(e.other) {
                    continue;
                }
                let f = self.graph.node(e.other).file;
                if f != own {
                    files.insert(f);
                }
                queue.push_back((e.other, depth + 1));
            }
        }
        files.into_iter().collect()
    }

    fn direct(&self, slot: u32, outgoing: bool, kinds: &[EdgeKind]) -> Vec<u32> {
        let list = if outgoing {
            self.graph.out_edges(slot)
        } else {
            self.graph.in_edges(slot)
        };
        list.iter()
            .filter(|e| kinds.contains(&e.kind))
            .map(|e| e.other)
            .collect()
    }

    pub fn context(&self, slot: u32, limit: usize) -> Option<Context> {
        let n = self.graph.get(slot)?;
        let symbol_kinds = [
            EdgeKind::Calls,
            EdgeKind::References,
            EdgeKind::Inherits,
            EdgeKind::Implements,
        ];
        let file = if n.kind.is_symbol() { n.file } else { slot };
        let (callers, callees) = if n.kind.is_symbol() {
            // A class's callers include its members' callers.
            let mut inn = Vec::new();
            let mut out = Vec::new();
            let mut members = self.graph.file_symbols(file);
            members.retain(|&m| m == slot || self.is_inside(m, slot));
            for m in members {
                inn.extend(
                    self.direct(m, false, &symbol_kinds)
                        .into_iter()
                        .filter(|&o| !self.is_inside(o, slot) && o != slot),
                );
                out.extend(
                    self.direct(m, true, &symbol_kinds)
                        .into_iter()
                        .filter(|&o| !self.is_inside(o, slot) && o != slot),
                );
            }
            (inn, out)
        } else {
            (Vec::new(), Vec::new())
        };
        let importers = if n.kind == NodeKind::Dir {
            Vec::new()
        } else {
            self.direct(file, false, &[EdgeKind::Imports])
        };
        let imports = if n.kind == NodeKind::Dir {
            Vec::new()
        } else {
            self.direct(file, true, &[EdgeKind::Imports])
        };
        let blast = self.blast_files(slot);
        let tests: Vec<u32> = blast
            .iter()
            .copied()
            .filter(|&f| self.graph.node(f).flags & flags::TEST != 0)
            .collect();
        Some(Context {
            callers: self.refs(callers, limit),
            callees: self.refs(callees, limit),
            importers: self.refs(importers, limit),
            imports: self.refs(imports, limit),
            tests: self.refs(tests, limit),
            blast_count: blast.len() as u32,
            blast: self.refs(blast, limit),
        })
    }

    /// Whether `slot` is nested inside `container` (a member of a class, say).
    fn is_inside(&self, mut slot: u32, container: u32) -> bool {
        while slot != NONE {
            let p = self.graph.node(slot).parent;
            if p == container {
                return true;
            }
            if p == NONE || !self.graph.node(p).kind.is_symbol() {
                return false;
            }
            slot = p;
        }
        false
    }

    /// Cyclomatic complexity per symbol slot of a file (functions and methods), read on demand
    /// and cached until the file's content changes.
    pub fn complexity(&self, file: u32) -> HashMap<u32, u32> {
        let Some(path) = self.graph.path(file) else {
            return HashMap::new();
        };
        let hash = self.meta_hash(file);
        if let Some((h, cached)) = self.complexity_cache.lock().get(&file) {
            if *h == hash {
                return cached.clone();
            }
        }
        let lang = self.graph.node(file).lang;
        let by_line: HashMap<u32, u32> = std::fs::read(self.root.join(path))
            .map(|src| {
                compass_parse::complexity::complexity(lang, &src)
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();
        let mut out = HashMap::new();
        for s in self.graph.file_symbols(file) {
            let n = self.graph.node(s);
            if matches!(n.kind, NodeKind::Function | NodeKind::Method) {
                if let Some(&c) = by_line.get(&n.start_line) {
                    out.insert(s, c);
                }
            }
        }
        self.complexity_cache
            .lock()
            .insert(file, (hash, out.clone()));
        out
    }

    /// CodeLens data for every function, method and class of a file.
    pub fn lenses(&self, path: &str) -> Vec<Lens> {
        let Some(file) = self.graph.slot_of_path(path) else {
            return Vec::new();
        };
        let cx = self.complexity(file);
        let kinds = [
            EdgeKind::Calls,
            EdgeKind::References,
            EdgeKind::Inherits,
            EdgeKind::Implements,
        ];
        self.graph
            .file_symbols(file)
            .into_iter()
            .filter(|&s| {
                matches!(
                    self.graph.node(s).kind,
                    NodeKind::Function | NodeKind::Method | NodeKind::Class
                )
            })
            .map(|s| {
                let callers: HashSet<u32> = self
                    .graph
                    .in_edges(s)
                    .iter()
                    .filter(|e| kinds.contains(&e.kind))
                    .map(|e| e.other)
                    .collect();
                let blast = self.blast_files(s);
                let tests = blast
                    .iter()
                    .filter(|&&f| self.graph.node(f).flags & flags::TEST != 0)
                    .count() as u32;
                Lens {
                    slot: s,
                    line: self.graph.node(s).start_line,
                    callers: callers.len() as u32,
                    blast: blast.len() as u32,
                    tests,
                    complexity: cx.get(&s).copied().unwrap_or(0),
                }
            })
            .collect()
    }
}
