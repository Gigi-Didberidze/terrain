//! Resolution: turns extracted facts into graph edges.
//!
//! Imports resolve to files per language (see `imports.rs`). References resolve by name, taking
//! the first non-empty tier: same file, imported files, same directory (package), and finally a
//! unique definition anywhere in the same language family. All edges are `syntactic`; ambiguous
//! names (more than three candidates in the winning tier) produce no edge rather than a guess.

use crate::imports::{self, Ctx, FileIndex};
use crate::project::{join, Project};
use compass_core::model::*;
use compass_core::{Graph, SymbolSpec};
use compass_parse::{FileFacts, RefKind};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Prefix for Tier 3 path mentions stored among imports.
pub const PATH_PREFIX: &str = "path:";
/// Above this many candidates in the winning tier a name is ambiguous: no edge.
const MAX_CANDIDATES: usize = 3;

#[derive(Default)]
pub struct Semantic {
    pub project: Project,
    pub files: FileIndex,
    facts: HashMap<u32, Arc<FileFacts>>,
    /// Symbol slots of each file, aligned with `facts.defs`.
    syms: HashMap<u32, Vec<u32>>,
    defs: HashMap<Box<str>, Vec<u32>>,
    ref_files: HashMap<Box<str>, HashSet<u32>>,
    namespaces: HashMap<String, Vec<String>>,
    pub imports_of: HashMap<u32, Vec<u32>>,
    /// Import specifiers that resolved to nothing in the workspace, per file (external or missing).
    pub unresolved: HashMap<u32, u32>,
}

/// Edges computed for one file, applied later under the write lock.
pub struct FileEdges {
    pub file: u32,
    pub imports: Vec<u32>,
    pub edges: Vec<(u32, u32, EdgeKind, f32)>,
    pub unresolved: u32,
}

fn family(l: Lang) -> u8 {
    match l {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => 1,
        Lang::C | Lang::Cpp => 2,
        Lang::Java | Lang::Kotlin => 3,
        other => 10 + other as u8,
    }
}

fn compatible(r: RefKind, k: NodeKind) -> bool {
    match r {
        RefKind::Call => matches!(k, NodeKind::Function | NodeKind::Method | NodeKind::Class),
        RefKind::Type | RefKind::Inherits | RefKind::Implements => {
            matches!(k, NodeKind::Class | NodeKind::Type)
        }
    }
}

fn edge_kind(r: RefKind) -> EdgeKind {
    match r {
        RefKind::Call => EdgeKind::Calls,
        RefKind::Type => EdgeKind::References,
        RefKind::Inherits => EdgeKind::Inherits,
        RefKind::Implements => EdgeKind::Implements,
    }
}

impl Semantic {
    pub fn facts(&self, file: u32) -> Option<&Arc<FileFacts>> {
        self.facts.get(&file)
    }

    pub fn has_facts(&self, file: u32) -> bool {
        self.facts.contains_key(&file)
    }

    pub fn files_with_facts(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.facts.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Replaces a file's facts and symbols. Returns the definition names whose presence changed
    /// (added or removed), which decides which other files must be re-resolved.
    pub fn set_facts(
        &mut self,
        g: &mut Graph,
        file: u32,
        facts: Option<Arc<FileFacts>>,
        is_test: bool,
    ) -> HashSet<Box<str>> {
        let path = g.path(file).unwrap_or("").to_string();
        let old = self.facts.remove(&file);
        let old_syms = self.syms.remove(&file).unwrap_or_default();
        let mut before: HashMap<(&str, NodeKind), u32> = HashMap::new();
        if let Some(o) = &old {
            for (d, &slot) in o.defs.iter().zip(&old_syms) {
                *before.entry((d.name.as_str(), d.kind)).or_default() += 1;
                if let Some(v) = self.defs.get_mut(d.name.as_str()) {
                    v.retain(|&s| s != slot);
                    if v.is_empty() {
                        self.defs.remove(d.name.as_str());
                    }
                }
            }
            for r in &o.refs {
                if let Some(set) = self.ref_files.get_mut(r.name.as_str()) {
                    set.remove(&file);
                    if set.is_empty() {
                        self.ref_files.remove(r.name.as_str());
                    }
                }
            }
            for ns in &o.namespaces {
                if let Some(v) = self.namespaces.get_mut(ns) {
                    v.retain(|p| p != &path);
                }
            }
        }
        let facts = facts.unwrap_or_default();
        let specs: Vec<SymbolSpec> = facts
            .defs
            .iter()
            .map(|d| SymbolSpec {
                kind: d.kind,
                name: d.name.clone(),
                parent: d.parent.map(|p| p as usize),
                start_line: d.start_line,
                end_line: d.end_line,
                flags: (if d.exported { flags::EXPORTED } else { 0 })
                    | (if d.entry || is_test { flags::ENTRY } else { 0 })
                    | (if is_test { flags::TEST } else { 0 }),
            })
            .collect();
        let diff = g.replace_symbols(file, &specs);
        g.set_flag(file, flags::ENTRY, facts.entry || is_test);
        let mut after: HashMap<(&str, NodeKind), u32> = HashMap::new();
        for (d, &slot) in facts.defs.iter().zip(&diff.slots) {
            *after.entry((d.name.as_str(), d.kind)).or_default() += 1;
            self.defs
                .entry(d.name.as_str().into())
                .or_default()
                .push(slot);
        }
        for r in &facts.refs {
            self.ref_files
                .entry(r.name.as_str().into())
                .or_default()
                .insert(file);
        }
        for ns in &facts.namespaces {
            self.namespaces
                .entry(ns.clone())
                .or_default()
                .push(path.clone());
        }
        let mut changed: HashSet<Box<str>> = HashSet::new();
        for (k, n) in &before {
            if after.get(k) != Some(n) {
                changed.insert(k.0.into());
            }
        }
        for (k, n) in &after {
            if before.get(k) != Some(n) {
                changed.insert(k.0.into());
            }
        }
        self.syms.insert(file, diff.slots);
        let empty = facts.defs.is_empty() && facts.refs.is_empty() && facts.imports.is_empty();
        if !empty || old.is_some() {
            self.facts.insert(file, facts);
        }
        changed
    }

    /// Forgets a deleted file (the graph already removed its nodes and edges).
    pub fn remove_file(&mut self, g: &mut Graph, file: u32) -> HashSet<Box<str>> {
        let changed = if self.facts.contains_key(&file) {
            self.set_facts(g, file, None, false)
        } else {
            HashSet::new()
        };
        self.facts.remove(&file);
        self.syms.remove(&file);
        self.imports_of.remove(&file);
        self.unresolved.remove(&file);
        changed
    }

    /// Files that must be re-resolved after definitions named `names` appeared or disappeared.
    pub fn referencing(&self, names: &HashSet<Box<str>>) -> HashSet<u32> {
        let mut out = HashSet::new();
        for n in names {
            if let Some(f) = self.ref_files.get(n) {
                out.extend(f.iter().copied());
            }
        }
        out
    }

    /// Every import of a file with the workspace files it resolves to (for accuracy checks).
    pub fn import_targets(
        &self,
        g: &Graph,
        file: u32,
    ) -> Vec<(compass_parse::Import, Vec<String>)> {
        let (Some(facts), Some(from)) = (self.facts.get(&file), g.path(file)) else {
            return Vec::new();
        };
        let ctx = Ctx {
            idx: &self.files,
            project: &self.project,
            namespaces: &self.namespaces,
        };
        let lang = g.node(file).lang;
        facts
            .imports
            .iter()
            .filter(|i| !i.path.starts_with(PATH_PREFIX))
            .map(|i| (i.clone(), imports::resolve(lang, from, i, &ctx)))
            .collect()
    }

    /// Computes a file's edges without mutating anything (safe to run in parallel).
    pub fn compute(&self, g: &Graph, file: u32) -> Option<FileEdges> {
        let facts = self.facts.get(&file)?;
        let syms = self.syms.get(&file)?;
        let from = g.path(file)?;
        let lang = g.node(file).lang;
        let ctx = Ctx {
            idx: &self.files,
            project: &self.project,
            namespaces: &self.namespaces,
        };
        let mut imports = Vec::new();
        let mut unresolved = 0;
        let mut edges = Vec::new();
        for imp in &facts.imports {
            if let Some(p) = imp.path.strip_prefix(PATH_PREFIX) {
                // Tier 3 path mention: relative to the file, then to the workspace root.
                let p = p.trim_start_matches("./");
                let dir = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                let cands = [join(dir, p), join("", p)];
                if let Some(t) = cands
                    .iter()
                    .find_map(|c| g.slot_of_path(c.trim_end_matches('/')))
                {
                    if t != file && g.node(t).kind == NodeKind::File {
                        edges.push((file, t, EdgeKind::References, 1.0));
                    }
                }
                continue;
            }
            let targets = imports::resolve(lang, from, imp, &ctx);
            if targets.is_empty() {
                unresolved += 1;
            }
            let w = 1.0 / targets.len().max(1) as f32;
            for t in targets {
                if let Some(slot) = g.slot_of_path(&t) {
                    edges.push((file, slot, EdgeKind::Imports, w));
                    imports.push(slot);
                }
            }
        }
        imports.sort_unstable();
        imports.dedup();
        let imported: HashSet<u32> = imports.iter().copied().collect();
        let dir = g.node(file).parent;
        let fam = family(lang);
        for r in &facts.refs {
            let Some(cands) = self.defs.get(r.name.as_str()) else {
                continue;
            };
            let src = r
                .from
                .and_then(|i| syms.get(i as usize).copied())
                .unwrap_or(file);
            let ok = |&&s: &&u32| {
                let n = g.node(s);
                n.alive() && compatible(r.kind, n.kind) && s != src
            };
            let tiers: [Box<dyn Fn(u32) -> bool>; 4] = [
                Box::new(|s| g.node(s).file == file),
                Box::new(|s| imported.contains(&g.node(s).file)),
                Box::new(|s| g.node(g.node(s).file).parent == dir),
                Box::new(|s| family(g.node(s).lang) == fam),
            ];
            for (i, tier) in tiers.iter().enumerate() {
                let hit: Vec<u32> = cands
                    .iter()
                    .filter(ok)
                    .copied()
                    .filter(|&s| tier(s))
                    .collect();
                if hit.is_empty() {
                    continue;
                }
                // The global tier only counts when the name is unique.
                let limit = if i == 3 { 1 } else { MAX_CANDIDATES };
                if hit.len() <= limit {
                    let w = if i == 3 { 0.5 } else { 1.0 / hit.len() as f32 };
                    for d in hit {
                        edges.push((src, d, edge_kind(r.kind), w));
                    }
                }
                break;
            }
        }
        Some(FileEdges {
            file,
            imports,
            edges,
            unresolved,
        })
    }

    /// Replaces a file's syntactic edges with freshly computed ones.
    pub fn apply(&mut self, g: &mut Graph, fe: FileEdges) {
        let pred = |e: &EdgeRec| e.conf == Confidence::Syntactic && e.kind.is_dependency();
        g.clear_file_out_edges(fe.file, pred);
        for (s, d, k, w) in fe.edges {
            if g.get(s).is_some() && g.get(d).is_some() {
                g.add_edge(s, d, k, Confidence::Syntactic, w);
            }
        }
        self.unresolved.insert(fe.file, fe.unresolved);
        self.imports_of.insert(fe.file, fe.imports);
    }

    /// Resolves `files` in parallel and applies the results.
    pub fn resolve_files(&mut self, g: &mut Graph, files: &[u32]) {
        let results: Vec<FileEdges> = {
            let me = &*self;
            let gr = &*g;
            files
                .par_iter()
                .filter_map(|&f| me.compute(gr, f))
                .collect()
        };
        for fe in results {
            self.apply(g, fe);
        }
    }
}
