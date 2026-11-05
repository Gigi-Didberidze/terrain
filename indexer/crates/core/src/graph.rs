//! The in-memory graph: the single source of truth every view queries.
//!
//! Nodes live in an arena of slots. A slot is never reused for a different node within one
//! indexer session, so slot numbers are safe handles for the UI. Symbol slots are reused only
//! when a re-parsed file yields the same stable [`NodeId`], which keeps inbound edges valid.

use crate::model::*;
use std::collections::HashMap;

#[derive(Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    by_id: HashMap<NodeId, u32>,
    by_path: HashMap<Box<str>, u32>,
    paths: HashMap<u32, Box<str>>,
    children: Vec<Vec<u32>>,
    out: Vec<Vec<EdgeRec>>,
    inn: Vec<Vec<EdgeRec>>,
    /// Bumped on any change.
    pub epoch: u64,
    /// Bumped when the treemap layout could change (files added/removed, LOC changed).
    pub layout_epoch: u64,
    alive_files: usize,
}

/// One symbol produced by extraction, before it is placed in the graph.
#[derive(Clone, Debug)]
pub struct SymbolSpec {
    pub kind: NodeKind,
    pub name: String,
    /// Index into the same spec list, or None for top-level symbols (parent = file).
    pub parent: Option<usize>,
    pub start_line: u32,
    pub end_line: u32,
    pub flags: u16,
}

#[derive(Default, Debug)]
pub struct SymbolDiff {
    /// Slot for every spec, in spec order.
    pub slots: Vec<u32>,
    pub added: Vec<u32>,
    pub removed: Vec<u32>,
}

impl Graph {
    pub fn new(root_name: &str) -> Self {
        let mut g = Graph::default();
        let root = g.push(Node {
            id: NodeId::of("", ""),
            kind: NodeKind::Dir,
            lang: Lang::Unknown,
            flags: flags::ALIVE,
            name: root_name.into(),
            parent: NONE,
            file: NONE,
            start_line: 0,
            end_line: 0,
            loc: 0,
        });
        g.by_path.insert("".into(), root);
        g.paths.insert(root, "".into());
        g
    }

    pub const ROOT: u32 = 0;

    fn push(&mut self, node: Node) -> u32 {
        let slot = self.nodes.len() as u32;
        self.by_id.insert(node.id, slot);
        self.nodes.push(node);
        self.children.push(Vec::new());
        self.out.push(Vec::new());
        self.inn.push(Vec::new());
        slot
    }

    pub fn node(&self, slot: u32) -> &Node {
        &self.nodes[slot as usize]
    }
    pub fn get(&self, slot: u32) -> Option<&Node> {
        self.nodes.get(slot as usize).filter(|n| n.alive())
    }
    pub fn slot_of(&self, id: NodeId) -> Option<u32> {
        self.by_id.get(&id).copied()
    }
    pub fn slot_of_path(&self, path: &str) -> Option<u32> {
        self.by_path.get(path).copied()
    }
    /// Workspace-relative path of a dir or file ("" for the root).
    pub fn path(&self, slot: u32) -> Option<&str> {
        self.paths.get(&slot).map(|p| &**p)
    }
    pub fn children(&self, slot: u32) -> &[u32] {
        &self.children[slot as usize]
    }
    pub fn out_edges(&self, slot: u32) -> &[EdgeRec] {
        &self.out[slot as usize]
    }
    pub fn in_edges(&self, slot: u32) -> &[EdgeRec] {
        &self.inn[slot as usize]
    }
    pub fn file_count(&self) -> usize {
        self.alive_files
    }
    pub fn files(&self) -> impl Iterator<Item = (u32, &str)> + '_ {
        self.paths.iter().filter_map(move |(&s, p)| {
            (self.nodes[s as usize].kind == NodeKind::File).then_some((s, &**p))
        })
    }

    /// Qualified name of a symbol within its file ("Class.method"), or the path for dirs/files.
    pub fn qualified_name(&self, slot: u32) -> String {
        let n = self.node(slot);
        if !n.kind.is_symbol() {
            return self.path(slot).unwrap_or("").to_string();
        }
        let mut parts = vec![&*n.name];
        let mut p = n.parent;
        while p != NONE && self.node(p).kind.is_symbol() {
            parts.push(&self.node(p).name);
            p = self.node(p).parent;
        }
        parts.reverse();
        parts.join(".")
    }

    fn ensure_dir(&mut self, path: &str) -> u32 {
        if let Some(&s) = self.by_path.get(path) {
            return s;
        }
        let (parent_path, name) = match path.rsplit_once('/') {
            Some((p, n)) => (p, n),
            None => ("", path),
        };
        let parent = self.ensure_dir(parent_path);
        let slot = self.push(Node {
            id: NodeId::of(path, ""),
            kind: NodeKind::Dir,
            lang: Lang::Unknown,
            flags: flags::ALIVE,
            name: name.into(),
            parent,
            file: NONE,
            start_line: 0,
            end_line: 0,
            loc: 0,
        });
        self.children[parent as usize].push(slot);
        self.by_path.insert(path.into(), slot);
        self.paths.insert(slot, path.into());
        slot
    }

    /// Adds a file or updates its LOC/flags. Returns its slot.
    pub fn upsert_file(&mut self, path: &str, loc: u32, lang: Lang, extra_flags: u16) -> u32 {
        self.epoch += 1;
        if let Some(&s) = self.by_path.get(path) {
            let n = &mut self.nodes[s as usize];
            if n.loc != loc {
                self.layout_epoch += 1;
            }
            n.loc = loc;
            n.end_line = loc;
            n.lang = lang;
            n.flags = flags::ALIVE | extra_flags;
            return s;
        }
        self.layout_epoch += 1;
        let (parent_path, name) = match path.rsplit_once('/') {
            Some((p, n)) => (p, n),
            None => ("", path),
        };
        let parent = self.ensure_dir(parent_path);
        let id = NodeId::of(path, "");
        let slot = self.push(Node {
            id,
            kind: NodeKind::File,
            lang,
            flags: flags::ALIVE | extra_flags,
            name: name.into(),
            parent,
            file: NONE,
            start_line: 1,
            end_line: loc,
            loc,
        });
        self.nodes[slot as usize].file = slot;
        self.children[parent as usize].push(slot);
        self.by_path.insert(path.into(), slot);
        self.paths.insert(slot, path.into());
        self.alive_files += 1;
        slot
    }

    /// Removes a file, its symbols and all their edges; prunes directories left empty.
    /// Returns the slots of nodes that had inbound edges from other files cut (for re-resolution).
    pub fn remove_file(&mut self, path: &str) -> Vec<u32> {
        let Some(slot) = self.by_path.get(path).copied() else {
            return Vec::new();
        };
        if self.nodes[slot as usize].kind != NodeKind::File {
            return Vec::new();
        }
        self.epoch += 1;
        self.layout_epoch += 1;
        let mut doomed = Vec::new();
        self.collect_subtree(slot, &mut doomed);
        let mut affected_sources = Vec::new();
        for &s in &doomed {
            for e in std::mem::take(&mut self.inn[s as usize]) {
                if self.nodes[e.other as usize].file != slot {
                    affected_sources.push(e.other);
                }
                self.out[e.other as usize].retain(|o| o.other != s);
            }
            for e in std::mem::take(&mut self.out[s as usize]) {
                self.inn[e.other as usize].retain(|i| i.other != s);
            }
            let n = &mut self.nodes[s as usize];
            n.flags &= !flags::ALIVE;
            self.by_id.remove(&n.id);
            self.children[s as usize].clear();
        }
        let parent = self.nodes[slot as usize].parent;
        self.children[parent as usize].retain(|&c| c != slot);
        self.by_path.remove(path);
        self.paths.remove(&slot);
        self.alive_files -= 1;
        self.prune_dir(parent);
        affected_sources.sort_unstable();
        affected_sources.dedup();
        affected_sources
    }

    fn prune_dir(&mut self, mut dir: u32) {
        while dir != Self::ROOT && self.children[dir as usize].is_empty() {
            let parent = self.nodes[dir as usize].parent;
            self.children[parent as usize].retain(|&c| c != dir);
            let n = &mut self.nodes[dir as usize];
            n.flags &= !flags::ALIVE;
            self.by_id.remove(&n.id);
            if let Some(p) = self.paths.remove(&dir) {
                self.by_path.remove(&p);
            }
            dir = parent;
        }
    }

    fn collect_subtree(&self, slot: u32, out: &mut Vec<u32>) {
        out.push(slot);
        for &c in &self.children[slot as usize] {
            self.collect_subtree(c, out);
        }
    }

    /// Replaces the symbols of a file. Symbols whose stable id survives keep their slot (and
    /// their inbound edges); removed symbols lose all edges.
    pub fn replace_symbols(&mut self, file: u32, specs: &[SymbolSpec]) -> SymbolDiff {
        self.epoch += 1;
        let path: Box<str> = self.paths[&file].clone();
        // Old symbols of this file.
        let mut old = Vec::new();
        for &c in &self.children[file as usize] {
            self.collect_subtree(c, &mut old);
        }
        let mut qnames: Vec<String> = Vec::with_capacity(specs.len());
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut diff = SymbolDiff::default();
        for s in specs.iter() {
            let base = match s.parent {
                Some(p) => format!("{}.{}", qnames[p], s.name),
                None => s.name.clone(),
            };
            // Overloads / redefinitions get an ordinal so ids stay unique and stable.
            let n = seen.entry(base.clone()).or_insert(0);
            let q = if *n == 0 {
                base.clone()
            } else {
                format!("{base}#{n}")
            };
            *n += 1;
            qnames.push(base);
            let id = NodeId::of(&path, &q);
            let parent = s.parent.map(|p| diff.slots[p]).unwrap_or(file);
            let slot = match self.by_id.get(&id).copied() {
                Some(slot) if self.nodes[slot as usize].file == file => {
                    let node = &mut self.nodes[slot as usize];
                    node.kind = s.kind;
                    node.parent = parent;
                    node.start_line = s.start_line;
                    node.end_line = s.end_line;
                    node.loc = s.end_line.saturating_sub(s.start_line) + 1;
                    node.flags = flags::ALIVE | s.flags;
                    slot
                }
                _ => {
                    let lang = self.nodes[file as usize].lang;
                    let slot = self.push(Node {
                        id,
                        kind: s.kind,
                        lang,
                        flags: flags::ALIVE | s.flags,
                        name: s.name.as_str().into(),
                        parent,
                        file,
                        start_line: s.start_line,
                        end_line: s.end_line,
                        loc: s.end_line.saturating_sub(s.start_line) + 1,
                    });
                    diff.added.push(slot);
                    slot
                }
            };
            diff.slots.push(slot);
        }
        // Rebuild containment for this file.
        let keep: std::collections::HashSet<u32> = diff.slots.iter().copied().collect();
        for &o in &old {
            self.children[o as usize].clear();
        }
        self.children[file as usize].clear();
        for &s in &diff.slots {
            let p = self.nodes[s as usize].parent;
            self.children[p as usize].push(s);
        }
        for o in old {
            if keep.contains(&o) {
                continue;
            }
            for e in std::mem::take(&mut self.inn[o as usize]) {
                self.out[e.other as usize].retain(|x| x.other != o);
            }
            for e in std::mem::take(&mut self.out[o as usize]) {
                self.inn[e.other as usize].retain(|x| x.other != o);
            }
            let n = &mut self.nodes[o as usize];
            n.flags &= !flags::ALIVE;
            self.by_id.remove(&n.id);
            diff.removed.push(o);
        }
        diff
    }

    /// All alive symbol slots in a file (depth-first, parents before children).
    pub fn file_symbols(&self, file: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for &c in &self.children[file as usize] {
            self.collect_subtree(c, &mut out);
        }
        out
    }

    pub fn add_edge(&mut self, src: u32, dst: u32, kind: EdgeKind, conf: Confidence, weight: f32) {
        if src == dst {
            return;
        }
        // Merge parallel edges of the same kind and confidence.
        if let Some(e) = self.out[src as usize]
            .iter_mut()
            .find(|e| e.other == dst && e.kind == kind && e.conf == conf)
        {
            e.weight += weight;
            if let Some(i) = self.inn[dst as usize]
                .iter_mut()
                .find(|e| e.other == src && e.kind == kind && e.conf == conf)
            {
                i.weight += weight;
            }
            return;
        }
        self.out[src as usize].push(EdgeRec {
            other: dst,
            kind,
            conf,
            weight,
        });
        self.inn[dst as usize].push(EdgeRec {
            other: src,
            kind,
            conf,
            weight,
        });
        self.epoch += 1;
    }

    /// Removes outbound edges of `src` matching `pred`.
    pub fn clear_out_edges(&mut self, src: u32, pred: impl Fn(&EdgeRec) -> bool) {
        let out = std::mem::take(&mut self.out[src as usize]);
        let mut kept = Vec::with_capacity(out.len());
        for e in out {
            if pred(&e) {
                self.inn[e.other as usize]
                    .retain(|i| !(i.other == src && i.kind == e.kind && i.conf == e.conf));
            } else {
                kept.push(e);
            }
        }
        self.out[src as usize] = kept;
        self.epoch += 1;
    }

    /// Clears outbound edges from a file and all its symbols matching `pred`.
    pub fn clear_file_out_edges(&mut self, file: u32, pred: impl Fn(&EdgeRec) -> bool + Copy) {
        self.clear_out_edges(file, pred);
        for s in self.file_symbols(file) {
            self.clear_out_edges(s, pred);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: NodeKind, name: &str, parent: Option<usize>) -> SymbolSpec {
        SymbolSpec {
            kind,
            name: name.into(),
            parent,
            start_line: 1,
            end_line: 3,
            flags: 0,
        }
    }

    #[test]
    fn files_create_and_prune_dirs() {
        let mut g = Graph::new("repo");
        let f = g.upsert_file("a/b/c.rs", 10, Lang::Rust, 0);
        assert_eq!(g.path(f), Some("a/b/c.rs"));
        let b = g.slot_of_path("a/b").unwrap();
        assert_eq!(g.children(b), &[f]);
        g.remove_file("a/b/c.rs");
        assert!(g.slot_of_path("a").is_none());
        assert!(g.children(Graph::ROOT).is_empty());
        assert_eq!(g.file_count(), 0);
    }

    #[test]
    fn replacing_symbols_keeps_slots_and_inbound_edges() {
        let mut g = Graph::new("repo");
        let a = g.upsert_file("a.py", 10, Lang::Python, 0);
        let b = g.upsert_file("b.py", 10, Lang::Python, 0);
        let d1 = g.replace_symbols(
            a,
            &[
                spec(NodeKind::Class, "K", None),
                spec(NodeKind::Method, "m", Some(0)),
            ],
        );
        let db = g.replace_symbols(b, &[spec(NodeKind::Function, "f", None)]);
        g.add_edge(
            db.slots[0],
            d1.slots[1],
            EdgeKind::Calls,
            Confidence::Syntactic,
            1.0,
        );
        // Re-parse a.py: K.m survives, a new function appears.
        let d2 = g.replace_symbols(
            a,
            &[
                spec(NodeKind::Class, "K", None),
                spec(NodeKind::Method, "m", Some(0)),
                spec(NodeKind::Function, "g", None),
            ],
        );
        assert_eq!(d2.slots[1], d1.slots[1]);
        assert_eq!(d2.added.len(), 1);
        assert_eq!(g.in_edges(d1.slots[1]).len(), 1);
        assert_eq!(g.qualified_name(d2.slots[1]), "K.m");
        // Re-parse without K.m: the edge disappears from both ends.
        let d3 = g.replace_symbols(a, &[spec(NodeKind::Function, "g", None)]);
        assert_eq!(d3.removed.len(), 2);
        assert!(g.out_edges(db.slots[0]).is_empty());
    }

    #[test]
    fn duplicate_names_get_distinct_stable_ids() {
        let mut g = Graph::new("repo");
        let a = g.upsert_file("a.c", 10, Lang::C, 0);
        let d = g.replace_symbols(
            a,
            &[
                spec(NodeKind::Function, "f", None),
                spec(NodeKind::Function, "f", None),
            ],
        );
        assert_ne!(d.slots[0], d.slots[1]);
        let again = g.replace_symbols(
            a,
            &[
                spec(NodeKind::Function, "f", None),
                spec(NodeKind::Function, "f", None),
            ],
        );
        assert_eq!(d.slots, again.slots);
    }

    #[test]
    fn removing_a_file_reports_cut_inbound_sources() {
        let mut g = Graph::new("repo");
        let a = g.upsert_file("a.ts", 1, Lang::TypeScript, 0);
        let b = g.upsert_file("b.ts", 1, Lang::TypeScript, 0);
        g.add_edge(b, a, EdgeKind::Imports, Confidence::Syntactic, 1.0);
        assert_eq!(g.remove_file("a.ts"), vec![b]);
        assert!(g.out_edges(b).is_empty());
    }
}
