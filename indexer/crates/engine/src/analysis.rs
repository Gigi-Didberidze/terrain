//! The Analyse stage: metrics computed from the graph after resolution.
//!
//! - Load-bearing score: PageRank over the file dependency graph and the symbol graph.
//! - Communities: Leiden on the undirected, weighted file graph (imports + calls + references +
//!   git co-change). The 8 largest communities get colours, the rest stay grey. Files whose
//!   community is at home in another folder are listed as misplaced.
//! - Cycles: strongly connected components at file and directory level, each with the single
//!   edge whose removal breaks most of it.
//! - Dead-code candidates: symbols nothing depends on that are neither entry points nor exported.

use std::collections::HashMap;
use terrain_core::metrics::{self, Csr};
use terrain_core::model::*;
use terrain_core::Graph;

/// Colours available to the Structure lens (categorical palette of 8; 0 = grey).
pub const COMMUNITY_COLOURS: u32 = 8;
/// Cycles are listed only up to this many; the largest come first.
const MAX_CYCLES: usize = 200;
/// Weight of a co-change edge per unit of coupling degree (static edges weigh about 1 each).
pub const COCHANGE_WEIGHT: f32 = 2.0;
/// A folder needs this many files, and its dominant community this share of them, before a
/// member of another community counts as misplaced.
const MISPLACED_MIN_FILES: usize = 3;
const MISPLACED_MIN_SHARE: f32 = 0.5;

/// A file whose community lives mostly in another folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Misplaced {
    pub file: u32,
    /// The folder holding most of the file's community.
    pub home: u32,
    /// Share of the file's community in `home`.
    pub share: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Cycle {
    /// Members (file or directory slots).
    pub nodes: Vec<u32>,
    /// The dependency whose removal breaks most of the cycle: (from, to).
    pub break_edge: Option<(u32, u32)>,
}

#[derive(Default)]
pub struct Analysis {
    /// Graph epoch this analysis reflects.
    pub epoch: u64,
    /// Dense index → file slot, and back.
    pub files: Vec<u32>,
    index: HashMap<u32, u32>,
    /// File dependency graph (A → B: A depends on B) and its reverse.
    pub deps: Csr,
    pub dependents: Csr,
    /// Per slot (files and symbols): load-bearing percentile 0..=65535, community colour 0..=8.
    pub load: Vec<u16>,
    pub colour: Vec<u8>,
    /// Per slot: raw PageRank (files among files, symbols among symbols).
    pub rank: Vec<f32>,
    /// Community id per file index, and its member count.
    pub community: Vec<u32>,
    pub community_sizes: Vec<u32>,
    pub file_cycles: Vec<Cycle>,
    pub dir_cycles: Vec<Cycle>,
    /// Slots of symbols nothing depends on (and not entry points or exported), sorted.
    pub dead: Vec<u32>,
    /// Files that are part of an import cycle, sorted.
    pub in_cycle: Vec<u32>,
    /// Files whose community differs from their folder's, strongest first.
    pub misplaced: Vec<Misplaced>,
    pub millis: f64,
}

impl Analysis {
    pub fn file_index(&self, slot: u32) -> Option<u32> {
        self.index.get(&slot).copied()
    }

    /// Whether a static dependency links two files, in either direction.
    pub fn linked(&self, a: u32, b: u32) -> bool {
        let (Some(i), Some(j)) = (self.file_index(a), self.file_index(b)) else {
            return false;
        };
        self.deps.neighbors(i).any(|(t, _)| t == j) || self.deps.neighbors(j).any(|(t, _)| t == i)
    }

    pub fn is_dead(&self, slot: u32) -> bool {
        self.dead.binary_search(&slot).is_ok()
    }

    pub fn is_in_cycle(&self, slot: u32) -> bool {
        self.in_cycle.binary_search(&slot).is_ok()
    }

    /// Community id of a file; u32::MAX when unknown.
    pub fn community_of(&self, file: u32) -> u32 {
        self.file_index(file)
            .map(|i| self.community[i as usize])
            .unwrap_or(u32::MAX)
    }

    pub fn load_of(&self, slot: u32) -> u16 {
        self.load.get(slot as usize).copied().unwrap_or(0)
    }

    pub fn colour_of(&self, slot: u32) -> u8 {
        self.colour.get(slot as usize).copied().unwrap_or(0)
    }

    pub fn rank_of(&self, slot: u32) -> f32 {
        self.rank.get(slot as usize).copied().unwrap_or(0.0)
    }

    /// Files that transitively depend on `file` (depth ≤ `depth`), with their depth.
    pub fn file_blast(&self, file: u32, depth: u32, limit: usize) -> Vec<(u32, u32)> {
        let Some(i) = self.file_index(file) else {
            return Vec::new();
        };
        metrics::reachable(&self.dependents, i, depth, limit)
            .into_iter()
            .map(|(j, d)| (self.files[j as usize], d))
            .collect()
    }
}

/// Log-scaled position (0..=65535) of each value between the smallest and the largest. PageRank
/// is heavy-tailed: most files tie at the floor, so percentiles would light up everything that
/// has any dependents. On a log scale only what really holds the codebase up stands out.
fn log_scale(values: &[f64]) -> Vec<u16> {
    let lo = values
        .iter()
        .copied()
        .filter(|v| *v > 0.0)
        .fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(0.0, f64::max);
    if !lo.is_finite() || hi <= lo {
        return vec![0; values.len()];
    }
    let span = (hi / lo).ln();
    values
        .iter()
        .map(|&v| {
            if v <= lo {
                0
            } else {
                ((v / lo).ln() / span * 65535.0).round().clamp(0.0, 65535.0) as u16
            }
        })
        .collect()
}

/// Percentile (0..=65535) of each value among `values` (ties share the lower rank).
#[cfg(test)]
fn percentiles(values: &[f64]) -> Vec<u16> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut out = vec![0u16; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && values[order[j + 1]] == values[order[i]] {
            j += 1;
        }
        let p = if n == 1 {
            65535
        } else {
            (i as f64 / (n - 1) as f64 * 65535.0).round() as u16
        };
        for k in i..=j {
            out[order[k]] = p;
        }
        i = j + 1;
    }
    out
}

/// File-to-file import edges only: what "circular dependency" means to developers (and to
/// tools such as dependency-cruiser). Call-level mutual recursion across files is normal.
fn import_edges(g: &Graph, files: &[u32], index: &HashMap<u32, u32>) -> Vec<(u32, u32, f32)> {
    let mut edges = Vec::new();
    for (i, &f) in files.iter().enumerate() {
        for e in g.out_edges(f) {
            if e.kind == EdgeKind::Imports {
                if let Some(&t) = index.get(&e.other) {
                    edges.push((i as u32, t, e.weight));
                }
            }
        }
    }
    edges
}

/// Symbol-level dependency edges and file-level aggregation, in one pass over the graph.
fn file_edges(g: &Graph, index: &HashMap<u32, u32>) -> Vec<(u32, u32, f32)> {
    let mut edges = Vec::new();
    for (slot, n) in g.nodes.iter().enumerate() {
        if !n.alive() || n.kind == NodeKind::Dir {
            continue;
        }
        let Some(&from) = index.get(&n.file) else {
            continue;
        };
        for e in g.out_edges(slot as u32) {
            if !e.kind.is_dependency() {
                continue;
            }
            let other = g.node(e.other);
            if let Some(&to) = index.get(&other.file) {
                if to != from {
                    edges.push((from, to, e.weight));
                }
            }
        }
    }
    edges
}

/// Metrics over the graph. `cochange` holds (file slot, file slot, coupling degree) pairs from
/// git history; they join the community graph only.
pub fn analyse(g: &Graph, cochange: &[(u32, u32, f32)]) -> Analysis {
    let t0 = std::time::Instant::now();
    let mut files: Vec<(u32, &str)> = g.files().collect();
    // Index order by path: deterministic communities and cycles across runs.
    files.sort_by(|a, b| a.1.cmp(b.1));
    let files: Vec<u32> = files.into_iter().map(|(s, _)| s).collect();
    let index: HashMap<u32, u32> = files
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, i as u32))
        .collect();
    let n = files.len();
    let deps = Csr::from_edges(n, file_edges(g, &index));
    let dependents = deps.reversed();

    let slots = g.nodes.len();
    let mut load = vec![0u16; slots];
    let mut rank = vec![0f32; slots];
    let mut colour = vec![0u8; slots];

    // File load-bearing score.
    let file_rank = metrics::pagerank(&deps, 0.85, 50);
    for (i, p) in log_scale(&file_rank).into_iter().enumerate() {
        load[files[i] as usize] = p;
        rank[files[i] as usize] = file_rank[i] as f32;
    }

    // Symbol load-bearing score.
    let syms: Vec<u32> = (0..slots as u32)
        .filter(|&s| g.get(s).is_some_and(|n| n.kind.is_symbol()))
        .collect();
    let sym_index: HashMap<u32, u32> = syms
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, i as u32))
        .collect();
    let mut sym_edges = Vec::new();
    for (i, &s) in syms.iter().enumerate() {
        for e in g.out_edges(s) {
            if e.kind.is_dependency() {
                if let Some(&t) = sym_index.get(&e.other) {
                    sym_edges.push((i as u32, t, e.weight));
                }
            }
        }
    }
    let sym_rank = metrics::pagerank(&Csr::from_edges(syms.len(), sym_edges), 0.85, 30);
    for (i, p) in log_scale(&sym_rank).into_iter().enumerate() {
        load[syms[i] as usize] = p;
        rank[syms[i] as usize] = sym_rank[i] as f32;
    }

    // Communities over the undirected file graph; colours by size (largest first).
    let mut undirected: Vec<(u32, u32, f32)> = Vec::new();
    for s in 0..n as u32 {
        for (t, w) in deps.neighbors(s) {
            undirected.push((s, t, w));
            undirected.push((t, s, w));
        }
    }
    for &(a, b, d) in cochange {
        if let (Some(&i), Some(&j)) = (index.get(&a), index.get(&b)) {
            undirected.push((i, j, d * COCHANGE_WEIGHT));
            undirected.push((j, i, d * COCHANGE_WEIGHT));
        }
    }
    let community = metrics::leiden(&Csr::from_edges(n, undirected), 1.0, 8);
    let k = community.iter().map(|&c| c as usize + 1).max().unwrap_or(0);
    let mut sizes = vec![0u32; k];
    let mut lines = vec![0u64; k];
    for (i, &c) in community.iter().enumerate() {
        sizes[c as usize] += 1;
        lines[c as usize] += g.node(files[i]).loc as u64;
    }
    let mut by_size: Vec<usize> = (0..k).filter(|&c| sizes[c] > 1).collect();
    by_size.sort_by(|&a, &b| lines[b].cmp(&lines[a]).then(a.cmp(&b)));
    let mut colour_of_comm = vec![0u8; k];
    for (rank_i, &c) in by_size.iter().take(COMMUNITY_COLOURS as usize).enumerate() {
        colour_of_comm[c] = rank_i as u8 + 1;
    }
    for (i, &c) in community.iter().enumerate() {
        let col = colour_of_comm[c as usize];
        colour[files[i] as usize] = col;
        // Symbols take their file's community colour.
        for s in g.file_symbols(files[i]) {
            colour[s as usize] = col;
        }
    }

    // Cycles: over imports only.
    let imports = Csr::from_edges(n, import_edges(g, &files, &index));
    let file_cycles: Vec<Cycle> = metrics::strongly_connected(&imports)
        .into_iter()
        .take(MAX_CYCLES)
        .map(|comp| {
            let brk = metrics::best_break_edge(&imports, &comp, 400);
            Cycle {
                nodes: comp.iter().map(|&i| files[i as usize]).collect(),
                break_edge: brk.map(|(a, b)| (files[a as usize], files[b as usize])),
            }
        })
        .collect();
    let dir_cycles = dir_cycles(g, &files, &imports);
    let mut in_cycle: Vec<u32> = file_cycles
        .iter()
        .flat_map(|c: &Cycle| c.nodes.iter().copied())
        .collect();
    in_cycle.sort_unstable();
    in_cycle.dedup();

    // Dead-code candidates.
    let mut dead: Vec<u32> = syms
        .iter()
        .copied()
        .filter(|&s| {
            let n = g.node(s);
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Class | NodeKind::Type
            ) && n.flags & (flags::ENTRY | flags::EXPORTED | flags::TEST | flags::GENERATED) == 0
                && !g.in_edges(s).iter().any(|e| e.kind.is_dependency())
                && !is_conventional_hook(&n.name)
        })
        .collect();
    dead.sort_unstable();

    let misplaced = misplaced(g, &files, &deps, &dependents, &community, &sizes);

    Analysis {
        epoch: g.epoch,
        files,
        index,
        deps,
        dependents,
        load,
        colour,
        rank,
        community,
        community_sizes: sizes,
        file_cycles,
        dir_cycles,
        dead,
        in_cycle,
        misplaced,
        millis: t0.elapsed().as_secs_f64() * 1000.0,
    }
}

/// Files whose folder is dominated by one community while they belong to another that is at
/// home elsewhere. Singleton communities (files with no edges) are never misplaced.
fn misplaced(
    g: &Graph,
    files: &[u32],
    deps: &Csr,
    dependents: &Csr,
    community: &[u32],
    sizes: &[u32],
) -> Vec<Misplaced> {
    // Per folder: community → member count; per community: folder → member count.
    let mut by_dir: HashMap<u32, HashMap<u32, u32>> = HashMap::new();
    let mut by_comm: HashMap<u32, HashMap<u32, u32>> = HashMap::new();
    // Tests, generated and data files sit with what they test or describe, not with their
    // module; counting them would call source files misplaced into tests/.
    let counted = |f: u32| g.node(f).flags & (flags::TEST | flags::GENERATED | flags::DATA) == 0;
    for (i, &f) in files.iter().enumerate() {
        if !counted(f) {
            continue;
        }
        let dir = g.node(f).parent;
        let c = community[i];
        *by_dir.entry(dir).or_default().entry(c).or_default() += 1;
        *by_comm.entry(c).or_default().entry(dir).or_default() += 1;
    }
    let top = |m: &HashMap<u32, u32>| -> Option<(u32, u32)> {
        m.iter()
            .map(|(&k, &v)| (k, v))
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
    };
    let mut out = Vec::new();
    for (i, &f) in files.iter().enumerate() {
        let c = community[i];
        if sizes[c as usize] < 2 || !counted(f) {
            continue;
        }
        let dir = g.node(f).parent;
        let members = &by_dir[&dir];
        let total: u32 = members.values().sum();
        if (total as usize) < MISPLACED_MIN_FILES {
            continue;
        }
        let Some((dominant, n)) = top(members) else {
            continue;
        };
        if dominant == c || (n as f32) < MISPLACED_MIN_SHARE * total as f32 {
            continue;
        }
        let Some((home, m)) = top(&by_comm[&c]) else {
            continue;
        };
        // A file one folder below (or above) its module's home is ordinary nesting.
        if home == dir || is_ancestor(g, home, dir) || is_ancestor(g, dir, home) {
            continue;
        }
        let share = m as f32 / by_comm[&c].values().sum::<u32>() as f32;
        // More than half of the module there, and the file itself uses or is used by that
        // folder: a cluster reached only indirectly is not evidence of where a file belongs.
        let touches_home = deps
            .neighbors(i as u32)
            .chain(dependents.neighbors(i as u32))
            .any(|(t, _)| g.node(files[t as usize]).parent == home);
        if share > MISPLACED_MIN_SHARE && touches_home {
            out.push(Misplaced {
                file: f,
                home,
                share,
            });
        }
    }
    out.sort_by(|a, b| b.share.total_cmp(&a.share).then(a.file.cmp(&b.file)));
    out
}

fn is_ancestor(g: &Graph, a: u32, mut node: u32) -> bool {
    while node != NONE {
        node = g.node(node).parent;
        if node == a {
            return true;
        }
    }
    false
}

/// Names the runtime or framework calls implicitly; never reported as dead.
fn is_conventional_hook(name: &str) -> bool {
    (name.starts_with("__") && name.ends_with("__"))
        || matches!(
            name,
            "main"
                | "init"
                | "constructor"
                | "initialize"
                | "toString"
                | "equals"
                | "hashCode"
                | "dispose"
                | "Dispose"
                | "drop"
                | "fmt"
                | "default"
                | "new"
                | "setUp"
                | "tearDown"
                | "render"
                | "ngOnInit"
        )
}

/// Cycles between directories ("modules"): file dependencies lifted to their directories.
fn dir_cycles(g: &Graph, files: &[u32], deps: &Csr) -> Vec<Cycle> {
    let mut dirs: Vec<u32> = files.iter().map(|&f| g.node(f).parent).collect();
    dirs.sort_unstable();
    dirs.dedup();
    let di: HashMap<u32, u32> = dirs
        .iter()
        .enumerate()
        .map(|(i, &d)| (d, i as u32))
        .collect();
    let mut edges = Vec::new();
    for (i, &f) in files.iter().enumerate() {
        let from = di[&g.node(f).parent];
        for (t, w) in deps.neighbors(i as u32) {
            let to = di[&g.node(files[t as usize]).parent];
            if from != to {
                edges.push((from, to, w));
            }
        }
    }
    let dg = Csr::from_edges(dirs.len(), edges);
    metrics::strongly_connected(&dg)
        .into_iter()
        .take(MAX_CYCLES)
        .map(|comp| {
            let brk = metrics::best_break_edge(&dg, &comp, 400);
            Cycle {
                nodes: comp.iter().map(|&i| dirs[i as usize]).collect(),
                break_edge: brk.map(|(a, b)| (dirs[a as usize], dirs[b as usize])),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_scale_keeps_the_floor_dark_and_the_top_bright() {
        let v = log_scale(&[0.001, 0.001, 0.01, 0.1]);
        assert_eq!(v, vec![0, 0, 32768, 65535]);
    }

    #[test]
    fn percentiles_share_ties_and_span_the_range() {
        assert_eq!(
            percentiles(&[0.1, 0.5, 0.5, 0.9]),
            vec![0, 21845, 21845, 65535]
        );
        assert_eq!(percentiles(&[3.0]), vec![65535]);
    }

    /// Two tight modules, `a/` and `b/`; `a/stray.rs` only talks to `b/`.
    fn two_modules(stray_flags: u16) -> (Graph, u32) {
        let mut g = Graph::new("ws");
        let mut file = |p: &str, f: u16| g.upsert_file(p, 10, Lang::Rust, f);
        let a: Vec<u32> = (0..4).map(|i| file(&format!("a/a{i}.rs"), 0)).collect();
        let b: Vec<u32> = (0..4).map(|i| file(&format!("b/b{i}.rs"), 0)).collect();
        let stray = file("a/stray.rs", stray_flags);
        let nested = file("b/sub/helper.rs", 0);
        let mut link =
            |x: u32, y: u32| g.add_edge(x, y, EdgeKind::Imports, Confidence::Syntactic, 1.0);
        for m in [&a, &b] {
            for i in 0..m.len() {
                for j in 0..m.len() {
                    if i != j {
                        link(m[i], m[j]);
                    }
                }
            }
        }
        for &x in &b {
            link(stray, x);
            link(nested, x);
        }
        (g, stray)
    }

    #[test]
    fn misplaced_finds_files_whose_module_lives_elsewhere() {
        let (g, stray) = two_modules(0);
        let a = analyse(&g, &[]);
        let paths: Vec<&str> = a
            .misplaced
            .iter()
            .map(|m| g.path(m.file).unwrap())
            .collect();
        assert_eq!(
            paths,
            vec!["a/stray.rs"],
            "nested b/sub/helper.rs is ordinary nesting"
        );
        assert_eq!(g.path(a.misplaced[0].home), Some("b"));
        assert_eq!(a.misplaced[0].file, stray);
        // A test file next to what it tests is never misplaced.
        let (g, _) = two_modules(flags::TEST);
        assert!(analyse(&g, &[]).misplaced.is_empty());
    }

    #[test]
    fn cochange_pulls_files_into_one_community() {
        // Two unrelated files with no code edge end up together when they always change together.
        let mut g = Graph::new("ws");
        let x = g.upsert_file("x.sql", 10, Lang::Unknown, 0);
        let y = g.upsert_file("y.rs", 10, Lang::Rust, 0);
        let alone = analyse(&g, &[]);
        assert_ne!(alone.community_of(x), alone.community_of(y));
        let together = analyse(&g, &[(x, y, 0.9)]);
        assert_eq!(together.community_of(x), together.community_of(y));
    }
}
