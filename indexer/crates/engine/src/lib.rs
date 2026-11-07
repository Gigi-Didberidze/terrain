//! compass-engine: the indexing pipeline (discover → hash → parse → resolve → analyse → persist)
//! and the queries every view runs against the graph.

pub mod analysis;
pub mod confirm;
pub mod context;
pub mod history;
pub mod imports;
pub mod project;
pub mod resolve;
pub mod review;
pub mod scan;
pub mod search;
pub mod views;

use compass_core::layout::Layout;
use compass_core::model::*;
use compass_core::store::{FileRow, Store};
use compass_core::tiles::{self, LensValues};
use compass_core::Graph;
use compass_parse::FileFacts;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use resolve::Semantic;
use scan::{Discovered, DEFAULT_SIZE_CAP};
use search::SearchIndex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Files per batch for parsing and persisting (one SQLite transaction each).
pub const BATCH: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Discover,
    Hash,
    Parse,
    Resolve,
    Analyse,
    Persist,
    History,
    Idle,
}

/// Callbacks from the pipeline to whoever hosts it (the RPC server, tests, benchmarks).
pub trait Events: Send + Sync {
    fn progress(&self, phase: Phase, done: u64, total: u64);
    /// The graph changed; `layout` is true when tile geometry may have moved.
    fn changed(&self, epoch: u64, layout: bool);
    /// An owned copy for background work (debounced re-analysis).
    fn boxed(&self) -> Box<dyn Events>;
}

pub struct NoEvents;
impl Events for NoEvents {
    fn progress(&self, _: Phase, _: u64, _: u64) {}
    fn changed(&self, _: u64, _: bool) {}
    fn boxed(&self) -> Box<dyn Events> {
        Box::new(NoEvents)
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub size_cap: u64,
    /// Data files count as at most `layout::DATA_CAP_LINES` lines of area (setting
    /// compass.map.capDataFiles).
    pub cap_data_files: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            size_cap: DEFAULT_SIZE_CAP,
            cap_data_files: true,
        }
    }
}

pub struct Engine {
    pub root: PathBuf,
    pub options: Options,
    pub graph: Graph,
    pub search: SearchIndex,
    /// Last known content state of every file, indexed by slot; mirrored in the store.
    meta: Vec<FileMeta>,
    layout: Mutex<Option<Layout>>,
    store: Option<Mutex<Store>>,
    pub phase: Phase,
    pub timings: Timings,
    /// Set once any tile has been served (lets the warm start prioritise the first view).
    tiles_served: Arc<std::sync::atomic::AtomicBool>,
    /// Facts, name index and resolution state.
    pub sem: Semantic,
    /// Latest metrics (load-bearing, communities, cycles, dead code); replaced atomically.
    pub analysis: Arc<analysis::Analysis>,
    /// Bumped on every change; a scheduled re-analysis only runs if it is still current.
    analysis_gen: Arc<std::sync::atomic::AtomicU64>,
    /// Complexity per file, keyed by content hash (computed on demand).
    complexity_cache: Mutex<ComplexityCache>,
    /// Latest git history signals; replaced atomically after each mining run.
    pub history: Arc<history::History>,
    /// Mined commits and the repository; one mining run at a time.
    git: Arc<Mutex<GitState>>,
    /// What the current branch changed and its blast radius (Change lens, review list).
    pub review: Arc<review::Review>,
}

/// Commits mined so far (newest first) and the head they reach.
#[derive(Default)]
struct GitState {
    opened: bool,
    repo: Option<compass_git::Repo>,
    loaded: bool,
    head: Option<String>,
    commits: Vec<compass_core::store::CommitRow>,
}

#[derive(Clone, Debug, Default)]
pub struct Timings {
    pub warm_load_ms: f64,
    pub first_map_ms: Option<f64>,
    pub discover_ms: f64,
    pub hash_ms: f64,
    pub total_ms: f64,
}

pub type Shared = Arc<RwLock<Engine>>;

/// Per file: (content hash, complexity per symbol slot).
type ComplexityCache =
    std::collections::HashMap<u32, ([u8; 32], std::collections::HashMap<u32, u32>)>;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FileMeta {
    pub hash: [u8; 32],
    pub size: u64,
    pub mtime: i64,
}

#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub slot: u32,
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub path: String,
    pub qualified: String,
    pub lang: Lang,
    pub loc: u32,
    pub start_line: u32,
    pub end_line: u32,
    pub flags: u16,
    pub files: u32,
    pub rect: Option<compass_core::Rect>,
    pub parent: Option<u32>,
}

impl Engine {
    pub fn new(root: &Path, store: Option<Store>, options: Options) -> Engine {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace".into());
        Engine {
            root: root.to_path_buf(),
            options,
            graph: Graph::new(&name),
            search: SearchIndex::default(),
            meta: Vec::new(),
            layout: Mutex::new(None),
            store: store.map(Mutex::new),
            phase: Phase::Idle,
            timings: Timings::default(),
            tiles_served: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sem: Semantic::default(),
            analysis: Arc::new(analysis::Analysis::default()),
            analysis_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            complexity_cache: Mutex::new(std::collections::HashMap::new()),
            history: Arc::new(history::History::default()),
            git: Arc::new(Mutex::new(GitState::default())),
            review: Arc::new(review::Review::default()),
        }
    }

    pub fn shared(self) -> Shared {
        Arc::new(RwLock::new(self))
    }

    /// Adds or updates a file. `index` adds it to search right away; the warm start skips that
    /// and indexes everything in one parallel pass after the map is up.
    fn put_file(&mut self, row: &FileRow, index: bool) {
        // Derived from path and size on every insert, so an index stored by an older version
        // picks up the classification without re-reading unchanged files.
        let mut file_flags = row.flags & !flags::DATA;
        if scan::counts_as_data(
            &row.path,
            row.loc,
            row.lang,
            row.flags & flags::TOO_LARGE != 0,
        ) {
            file_flags |= flags::DATA;
        }
        let slot = self
            .graph
            .upsert_file(&row.path, row.loc, row.lang, file_flags) as usize;
        self.sem.files.add(&row.path);
        if self.meta.len() <= slot {
            self.meta.resize(slot + 1, FileMeta::default());
        }
        self.meta[slot] = FileMeta {
            hash: row.hash,
            size: row.size,
            mtime: row.mtime,
        };
        if index {
            let name = row.path.rsplit('/').next().unwrap_or(&row.path);
            self.search.upsert(slot as u32, name, &row.path);
        }
    }

    /// Content hash of a file (zeros if unknown).
    pub fn meta_hash(&self, file: u32) -> [u8; 32] {
        self.meta
            .get(file as usize)
            .map(|m| m.hash)
            .unwrap_or([0; 32])
    }

    /// Content state of a file on the map.
    pub fn meta_of(&self, path: &str) -> Option<&FileMeta> {
        let slot = self.graph.slot_of_path(path)?;
        (self.graph.node(slot).kind == NodeKind::File)
            .then(|| self.meta.get(slot as usize))
            .flatten()
    }

    /// Adds every file to the search index in one parallel pass.
    fn index_all_files(shared: &Shared) {
        let items: Vec<(u32, String)> = {
            let e = shared.read();
            e.graph.files().map(|(s, p)| (s, p.to_string())).collect()
        };
        let entries = search::SearchIndex::prepare(
            items
                .par_iter()
                .map(|(s, p)| (*s, p.rsplit('/').next().unwrap_or(p), p.as_str())),
        );
        shared.write().search.extend(entries);
    }

    /// Removes a file; returns definition names that disappeared with it.
    fn drop_file(&mut self, path: &str) -> std::collections::HashSet<Box<str>> {
        let mut changed = std::collections::HashSet::new();
        if let Some(slot) = self.graph.slot_of_path(path) {
            for s in self.graph.file_symbols(slot) {
                self.search.remove(s);
            }
            self.search.remove(slot);
            changed = self.sem.remove_file(&mut self.graph, slot);
        }
        self.sem.files.remove(path);
        self.graph.remove_file(path);
        changed
    }

    /// Installs a file's facts: symbols on the map, search entries, name index.
    /// Returns the definition names whose presence changed.
    fn apply_facts(
        &mut self,
        file: u32,
        facts: Option<Arc<FileFacts>>,
    ) -> std::collections::HashSet<Box<str>> {
        for s in self.graph.file_symbols(file) {
            self.search.remove(s);
        }
        let is_test = self.graph.node(file).flags & flags::TEST != 0;
        self.forget_confirmations(file);
        let changed = self.sem.set_facts(&mut self.graph, file, facts, is_test);
        let path = self.graph.path(file).unwrap_or("").to_string();
        for s in self.graph.file_symbols(file) {
            let name = self.graph.node(s).name.to_string();
            self.search.upsert(s, &name, &path);
        }
        changed
    }

    /// Re-reads project configuration (tsconfig, go.mod, Cargo.toml, ...) from the file list.
    fn refresh_project(&mut self) {
        let paths: Vec<String> = self.graph.files().map(|(_, p)| p.to_string()).collect();
        self.sem.project = project::Project::discover(&self.root, paths.iter().map(|s| s.as_str()));
    }

    /// Current file list as store rows.
    fn file_rows(&self) -> Vec<FileRow> {
        self.graph
            .files()
            .map(|(slot, path)| {
                let n = self.graph.node(slot);
                let m = self.meta.get(slot as usize).copied().unwrap_or_default();
                FileRow {
                    path: path.to_string(),
                    hash: m.hash,
                    size: m.size,
                    mtime: m.mtime,
                    loc: n.loc,
                    lang: n.lang,
                    flags: n.flags & !flags::ALIVE,
                }
            })
            .collect()
    }

    fn save_snapshot(&self) {
        if let Some(store) = &self.store {
            if let Err(e) = store.lock().save_snapshot(&self.file_rows()) {
                eprintln!("snapshot write failed: {e}");
            }
        }
    }

    fn persist(&self, rows: &[FileRow], facts: &[([u8; 32], Vec<u8>)], deleted: &[String]) {
        if let Some(store) = &self.store {
            if let Err(e) = store.lock().write_batch(rows, facts, deleted) {
                eprintln!("store write failed: {e}");
            }
        }
    }

    /// Runs `f` with an up-to-date layout.
    pub fn with_layout<R>(&self, f: impl FnOnce(&mut Layout) -> R) -> R {
        let mut guard = self.layout.lock();
        let cap = self
            .options
            .cap_data_files
            .then_some(compass_core::layout::DATA_CAP_LINES);
        if guard
            .as_ref()
            .is_none_or(|l| l.epoch != self.graph.layout_epoch || l.data_cap != cap)
        {
            *guard = Some(Layout::compute_with(&self.graph, cap));
        }
        f(guard.as_mut().unwrap())
    }

    /// One packed tile.
    pub fn tile(&self, z: u32, x: u32, y: u32) -> Vec<u8> {
        self.tiles_served
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.with_layout(|layout| {
            let items = tiles::query(&self.graph, layout, z, x, y, tiles::TILE_ITEM_LIMIT);
            let a = &self.analysis;
            let h = &self.history;
            let r = &self.review;
            tiles::pack(&self.graph, layout, z, x, y, &items, |slot| {
                let path = self.file_path_of(slot).unwrap_or("");
                let file = self.file_of(slot);
                let change = match file {
                    Some(f) if r.is_changed(f) => tiles::FLAG_CHANGED,
                    Some(f) if r.in_blast(f) => tiles::FLAG_BLAST,
                    _ => 0,
                };
                LensValues {
                    community: a.colour_of(slot),
                    load: a.load_of(slot),
                    owner: h.owner_lens(path),
                    churn: h.churn_lens(path),
                    change,
                }
            })
        })
    }

    /// The file slot a node belongs to (itself for files); None for directories.
    pub fn file_of(&self, slot: u32) -> Option<u32> {
        let n = self.graph.get(slot)?;
        match n.kind {
            NodeKind::Dir => None,
            NodeKind::File => Some(slot),
            _ => Some(n.file),
        }
    }

    /// The file a node belongs to (itself for files), as a path; None for directories.
    pub fn file_path_of(&self, slot: u32) -> Option<&str> {
        let n = self.graph.get(slot)?;
        match n.kind {
            NodeKind::Dir => None,
            NodeKind::File => self.graph.path(slot),
            _ => self.graph.path(n.file),
        }
    }

    /// Co-change partners of a file that still exist: (partner, its slot, statically linked).
    /// Not linked means a hidden dependency.
    pub fn partners(&self, path: &str) -> Vec<(history::Partner, u32, bool)> {
        let Some(slot) = self.graph.slot_of_path(path) else {
            return Vec::new();
        };
        self.history
            .partners(path)
            .iter()
            .filter_map(|p| {
                let other = self.graph.slot_of_path(&p.path)?;
                Some((p.clone(), other, self.analysis.linked(slot, other)))
            })
            .collect()
    }

    /// Coupled file pairs with no static edge between them, best evidence first (degree weighted
    /// by shared commits, so 18 shared commits outrank 5). Data and generated files are left out:
    /// snapshots and translations change together by construction.
    pub fn hidden_pairs(&self, limit: usize) -> Vec<(u32, u32, history::Partner)> {
        let quiet = |s: u32| self.graph.node(s).flags & (flags::DATA | flags::GENERATED) != 0;
        let mut out = Vec::new();
        for (path, partners) in &self.history.coupling {
            let Some(a) = self.graph.slot_of_path(path) else {
                continue;
            };
            for p in partners {
                if path.as_str() >= p.path.as_str() {
                    continue; // each pair once
                }
                let Some(b) = self.graph.slot_of_path(&p.path) else {
                    continue;
                };
                if !quiet(a) && !quiet(b) && !self.analysis.linked(a, b) {
                    out.push((a, b, p.clone()));
                }
            }
        }
        let evidence = |p: &history::Partner| p.degree * p.shared as f32 / (p.shared as f32 + 5.0);
        out.sort_by(|x, y| {
            evidence(&y.2)
                .total_cmp(&evidence(&x.2))
                .then(y.2.shared.cmp(&x.2.shared))
                .then(x.0.cmp(&y.0))
                .then(x.1.cmp(&y.1))
        });
        out.truncate(limit);
        out
    }

    /// Lines per author for one file at HEAD (blame), largest first.
    pub fn blame(&self, path: &str) -> Vec<(String, u32)> {
        let git = self.git.lock();
        let Some(repo) = &git.repo else {
            return Vec::new();
        };
        let authors = &self.history.authors;
        repo.blame(path, &|id| authors.get(id).cloned())
            .unwrap_or_default()
    }

    pub fn node_info(&self, slot: u32) -> Option<NodeInfo> {
        let n = self.graph.get(slot)?;
        let path = match n.kind {
            NodeKind::Dir | NodeKind::File => self.graph.path(slot).unwrap_or("").to_string(),
            _ => self.graph.path(n.file).unwrap_or("").to_string(),
        };
        let rect = self.with_layout(|l| l.rect_of(&self.graph, slot));
        let files = self.with_layout(|l| l.file_counts.get(slot as usize).copied().unwrap_or(0));
        Some(NodeInfo {
            slot,
            id: n.id,
            kind: n.kind,
            name: n.name.to_string(),
            qualified: self.graph.qualified_name(slot),
            path,
            lang: n.lang,
            loc: n.loc,
            start_line: n.start_line,
            end_line: n.end_line,
            flags: n.flags,
            files,
            rect,
            parent: (n.parent != NONE).then_some(n.parent),
        })
    }

    /// The deepest node at a file position: the innermost symbol containing `line`, else the file.
    pub fn locate(&self, path: &str, line: Option<u32>) -> Option<u32> {
        let file = self.graph.slot_of_path(path)?;
        let Some(line) = line else { return Some(file) };
        let mut best = file;
        let mut cur = file;
        'outer: loop {
            for &c in self.graph.children(cur) {
                let n = self.graph.node(c);
                if n.start_line <= line && line <= n.end_line {
                    best = c;
                    cur = c;
                    continue 'outer;
                }
            }
            break;
        }
        Some(best)
    }

    pub fn status(&self) -> (Phase, usize) {
        (self.phase, self.graph.file_count())
    }

    /// Nodes in `slot`'s subtree (itself included), capped so huge directories stay cheap.
    fn subtree(&self, slot: u32, cap: usize) -> Vec<u32> {
        let mut out = vec![slot];
        let mut i = 0;
        while i < out.len() && out.len() < cap {
            let s = out[i];
            out.extend_from_slice(self.graph.children(s));
            i += 1;
        }
        out.truncate(cap);
        out
    }

    /// The node an edge endpoint is shown as: symbols stay symbols when looking at a symbol;
    /// for files and directories the other side is shown at file level.
    fn display_node(&self, other: u32, from_kind: NodeKind) -> u32 {
        let n = self.graph.node(other);
        if from_kind.is_symbol() || !n.kind.is_symbol() {
            other
        } else {
            n.file
        }
    }

    /// Dependency edges of a node, aggregated per other endpoint, heaviest first.
    /// For files and directories this includes the edges of everything inside them.
    pub fn edges_of(&self, slot: u32, limit: usize) -> Vec<EdgeView> {
        let Some(n) = self.graph.get(slot) else {
            return Vec::new();
        };
        let kind = n.kind;
        // Containers (dirs, files, classes) count the edges of everything inside them: what holds
        // a class up is mostly the callers of its methods.
        let inside: Vec<u32> = self.subtree(slot, 50_000);
        let inside_set: HashSet<u32> = inside.iter().copied().collect();
        let mut agg: std::collections::HashMap<(u32, bool), EdgeView> =
            std::collections::HashMap::new();
        for &s in &inside {
            for (outgoing, list) in [
                (true, self.graph.out_edges(s)),
                (false, self.graph.in_edges(s)),
            ] {
                for e in list {
                    if !e.kind.is_dependency() || inside_set.contains(&e.other) {
                        continue;
                    }
                    let other = self.display_node(e.other, kind);
                    if inside_set.contains(&other) {
                        continue;
                    }
                    let v = agg.entry((other, outgoing)).or_insert(EdgeView {
                        other,
                        outgoing,
                        kind: e.kind,
                        conf: e.conf,
                        weight: 0.0,
                        count: 0,
                    });
                    v.weight += e.weight;
                    v.count += 1;
                    // Show the most certain confidence and the most specific kind seen.
                    if (e.conf as u8) == (Confidence::Semantic as u8) {
                        v.conf = Confidence::Semantic;
                    }
                    if v.kind == EdgeKind::Imports && e.kind != EdgeKind::Imports {
                        v.kind = e.kind;
                    }
                }
            }
        }
        let mut v: Vec<EdgeView> = agg.into_values().collect();
        v.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.other.cmp(&b.other)));
        v.truncate(limit);
        v
    }

    /// Number of distinct nodes this one depends on / is depended on by (at display level).
    pub fn edge_counts(&self, slot: u32) -> (u32, u32) {
        let v = self.edges_of(slot, usize::MAX);
        let outs = v.iter().filter(|e| e.outgoing).count() as u32;
        (v.len() as u32 - outs, outs)
    }

    /// Bundling path from `a` to `b`: rects of `a`'s ancestors up to (not including) their lowest
    /// common ancestor, then down to `b`. Hierarchical edge bundling uses their centres as
    /// control points, so edges that share structure share a route.
    pub fn bundle_path(&self, a: u32, b: u32) -> Vec<compass_core::Rect> {
        let chain = |mut s: u32| {
            let mut v = vec![s];
            while self.graph.node(s).parent != NONE {
                s = self.graph.node(s).parent;
                v.push(s);
            }
            v
        };
        let (ca, cb) = (chain(a), chain(b));
        let sb: HashSet<u32> = cb.iter().copied().collect();
        let lca = ca.iter().copied().find(|s| sb.contains(s));
        let mut nodes: Vec<u32> = ca.iter().copied().take_while(|&s| Some(s) != lca).collect();
        if let Some(l) = lca {
            nodes.push(l);
        }
        let down: Vec<u32> = cb.iter().copied().take_while(|&s| Some(s) != lca).collect();
        nodes.extend(down.into_iter().rev());
        self.with_layout(|l| {
            let mut out = Vec::with_capacity(nodes.len());
            for &s in &nodes {
                if s != Graph::ROOT && !l.is_collapsed(&self.graph, s) {
                    if let Some(r) = l.rect_of(&self.graph, s) {
                        out.push(r);
                    }
                }
            }
            out
        })
    }
}

/// One aggregated dependency of a node.
#[derive(Clone, Debug)]
pub struct EdgeView {
    pub other: u32,
    /// true: the node depends on `other`; false: `other` depends on the node.
    pub outgoing: bool,
    pub kind: EdgeKind,
    pub conf: Confidence,
    pub weight: f32,
    /// Underlying edges merged into this one.
    pub count: u32,
}

/// Full index run: warm start from the store, then bring it up to date with the workspace.
/// Returns the timings. Heavy work runs outside the write lock; results are merged in batches.
pub fn run_index(shared: &Shared, events: &dyn Events) -> Timings {
    let t0 = Instant::now();
    let (root, size_cap) = {
        let e = shared.read();
        (e.root.clone(), e.options.size_cap)
    };

    if let Some(store) = &shared.read().store {
        if let Err(e) = store
            .lock()
            .check_extractor_version(compass_parse::EXTRACTOR_VERSION)
        {
            eprintln!("store: {e}");
        }
    }

    // Warm start: everything we knew last time is on the map before we touch the disk.
    let rows = {
        let e = shared.read();
        e.store
            .as_ref()
            .and_then(|s| {
                let s = s.lock();
                s.load_snapshot().or_else(|| s.load_files().ok())
            })
            .unwrap_or_default()
    };
    let warm = !rows.is_empty();
    let trace = std::env::var_os("COMPASS_TRACE").is_some();
    if trace {
        eprintln!("warm: load_files {} rows {:.0} ms", rows.len(), ms(t0));
    }
    if warm {
        let tp = Instant::now();
        let mut e = shared.write();
        e.graph.reserve(rows.len());
        for r in &rows {
            e.put_file(r, false);
        }
        if trace {
            eprintln!("warm: put_file loop {:.0} ms", ms(tp));
        }
        // Lay out now so the first tile request doesn't pay for it.
        e.with_layout(|_| ());
        e.timings.warm_load_ms = ms(t0);
        e.timings.first_map_ms = Some(ms(t0));
        let epoch = e.graph.epoch;
        let served = e.tiles_served.clone();
        drop(e);
        events.changed(epoch, true);
        // Give the first view priority: reconciliation (search, discovery) competes for CPU and
        // the engine lock, so it waits until the first tiles went out, or 300 ms at most.
        let wait = Instant::now();
        while !served.load(std::sync::atomic::Ordering::Relaxed) && wait.elapsed().as_millis() < 300
        {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let ts = Instant::now();
        Engine::index_all_files(shared);
        if trace {
            eprintln!("warm: search index {:.0} ms", ms(ts));
        }
    }

    // Discover.
    set_phase(shared, Phase::Discover);
    let mut seen: HashSet<String> = HashSet::new();
    let mut to_scan: Vec<Discovered> = Vec::new();
    let mut last_emit = Instant::now();
    let discovered = scan::discover(&root, |batch| {
        let mut e = shared.write();
        for d in batch {
            let known = e.meta_of(&d.path).copied();
            let unchanged = known.is_some_and(|r| r.size == d.size && r.mtime == d.mtime);
            if known.is_none() {
                // Provisional node so the map is usable before hashing finishes.
                let lang = Lang::from_path(&d.path);
                let row = FileRow {
                    path: d.path.clone(),
                    hash: [0; 32],
                    size: d.size,
                    mtime: 0,
                    loc: scan::estimate_loc(d.size).max(1),
                    lang,
                    flags: 0,
                };
                e.put_file(&row, true);
            }
            if !unchanged {
                to_scan.push(d.clone());
            }
            seen.insert(d.path);
        }
        let n = seen.len() as u64;
        if e.timings.first_map_ms.is_none() {
            e.timings.first_map_ms = Some(ms(t0));
        }
        let epoch = e.graph.epoch;
        drop(e);
        events.progress(Phase::Discover, n, n);
        if last_emit.elapsed().as_millis() > 250 {
            events.changed(epoch, true);
            last_emit = Instant::now();
        }
    });
    {
        let mut e = shared.write();
        let gone: Vec<String> = e
            .graph
            .files()
            .filter(|(_, p)| !seen.contains(*p))
            .map(|(_, p)| p.to_string())
            .collect();
        for p in &gone {
            e.drop_file(p);
        }
        e.persist(&[], &[], &gone);
        e.timings.discover_ms = ms(t0);
        let epoch = e.graph.epoch;
        drop(e);
        events.changed(epoch, true);
    }

    // Hash: biggest files first to avoid tail latency.
    set_phase(shared, Phase::Hash);
    to_scan.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    let total = to_scan.len() as u64;
    let mut done = 0u64;
    events.progress(Phase::Hash, 0, total);
    for chunk in to_scan.chunks(BATCH) {
        let tc = Instant::now();
        let processed = process_chunk(shared, &root, chunk, size_cap);
        let scan_ms = ms(tc);
        let tm = Instant::now();
        let mut e = shared.write();
        let mut rows = Vec::with_capacity(processed.len());
        let mut new_facts = Vec::new();
        for p in processed {
            e.put_file(&p.row, true);
            let slot = e.graph.slot_of_path(&p.row.path).unwrap();
            e.apply_facts(slot, p.facts.clone());
            if let (Some(f), true) = (&p.facts, p.fresh) {
                new_facts.push((p.row.hash, f.encode()));
            }
            rows.push(p.row);
        }
        let merge_ms = ms(tm);
        let tp = Instant::now();
        e.persist(&rows, &new_facts, &[]);
        if trace {
            eprintln!(
                "hash batch: scan {scan_ms:.0} ms, merge {merge_ms:.0} ms, persist {:.0} ms",
                ms(tp)
            );
        }
        done += chunk.len() as u64;
        let epoch = e.graph.epoch;
        drop(e);
        events.progress(Phase::Hash, done, total);
        events.changed(epoch, true);
    }

    shared.write().timings.hash_ms = ms(t0);

    // Facts for files that did not change since last time come from the store.
    if warm {
        load_cached_facts(shared);
    }

    // Resolve: compute under the read lock, apply in batches so the map stays responsive.
    set_phase(shared, Phase::Resolve);
    shared.write().refresh_project();
    let all = shared.read().sem.files_with_facts();
    resolve_in_batches(shared, &all, events);
    if trace {
        eprintln!("resolve: {} files, total {:.0} ms", all.len(), ms(t0));
    }

    // Analyse: metrics over the resolved graph, computed under the read lock.
    set_phase(shared, Phase::Analyse);
    run_analysis(shared, events);

    // History: mine git (incrementally after the first run), then re-cluster with co-change.
    set_phase(shared, Phase::History);
    run_history(shared, events);
    run_review(shared, events);
    if trace {
        let a = shared.read().analysis.clone();
        eprintln!(
            "analyse: {:.0} ms ({} files, {} communities, {} file cycles, {} dir cycles, {} dead candidates)",
            a.millis,
            a.files.len(),
            a.community_sizes.len(),
            a.file_cycles.len(),
            a.dir_cycles.len(),
            a.dead.len()
        );
    }

    let mut e = shared.write();
    e.timings.total_ms = ms(t0);
    e.phase = Phase::Idle;
    e.save_snapshot();
    let _ = discovered;
    let t = e.timings.clone();
    drop(e);
    events.progress(Phase::Idle, total, total);
    t
}

/// Config files whose change can alter import resolution everywhere.
fn is_project_config(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "tsconfig.json"
            | "jsconfig.json"
            | "package.json"
            | "go.mod"
            | "Cargo.toml"
            | "composer.json"
            | "compile_commands.json"
    )
}

/// Incremental update for paths reported by the editor or the file watcher (saves, creates,
/// deletes, branch switches). Re-resolves only what the change can affect. Returns true if the
/// map changed.
pub fn apply_changes(shared: &Shared, paths: &[String], events: &dyn Events) -> bool {
    let (root, size_cap) = {
        let e = shared.read();
        (e.root.clone(), e.options.size_cap)
    };
    let existing: Vec<Discovered> = paths
        .iter()
        .filter_map(|p| {
            let m = std::fs::metadata(root.join(p))
                .ok()
                .filter(|m| m.is_file())?;
            Some(Discovered {
                path: p.clone(),
                size: m.len(),
                mtime: scan::mtime_ns(&m),
            })
        })
        .collect();
    let processed = process_chunk(shared, &root, &existing, size_cap);
    let present: HashSet<&str> = existing.iter().map(|d| d.path.as_str()).collect();
    let mut e = shared.write();
    let before = e.graph.layout_epoch;
    let mut rows = Vec::new();
    let mut new_facts = Vec::new();
    let mut gone = Vec::new();
    let mut changed_names: HashSet<Box<str>> = HashSet::new();
    let mut touched: HashSet<u32> = HashSet::new();
    let mut structural = false;
    for p in processed {
        if e.meta_of(&p.row.path)
            .is_some_and(|old| old.hash == p.row.hash)
        {
            continue;
        }
        structural |= e.graph.slot_of_path(&p.row.path).is_none() || is_project_config(&p.row.path);
        e.put_file(&p.row, true);
        let slot = e.graph.slot_of_path(&p.row.path).unwrap();
        changed_names.extend(e.apply_facts(slot, p.facts.clone()));
        touched.insert(slot);
        if let (Some(f), true) = (&p.facts, p.fresh) {
            new_facts.push((p.row.hash, f.encode()));
        }
        rows.push(p.row);
    }
    for p in paths {
        if !present.contains(p.as_str()) && e.meta_of(p).is_some() {
            changed_names.extend(e.drop_file(p));
            structural = true;
            gone.push(p.clone());
        }
    }
    if rows.is_empty() && gone.is_empty() {
        return false;
    }
    e.persist(&rows, &new_facts, &gone);
    let affected: Vec<u32> = if structural {
        // Files appeared or vanished, or project config changed: any import may resolve
        // differently now.
        e.refresh_project();
        e.sem.files_with_facts()
    } else {
        let mut a = e.sem.referencing(&changed_names);
        a.extend(touched);
        let mut v: Vec<u32> = a
            .into_iter()
            .filter(|&f| e.graph.get(f).is_some())
            .collect();
        v.sort_unstable();
        v
    };
    if affected.len() <= 2000 {
        let Engine { sem, graph, .. } = &mut *e;
        sem.resolve_files(graph, &affected);
    }
    let layout = e.graph.layout_epoch != before;
    drop(e);
    if affected.len() > 2000 {
        resolve_in_batches(shared, &affected, events);
    }
    // Read the epoch after re-resolution so notifications never go backwards.
    let epoch = shared.read().graph.epoch;
    events.changed(epoch, layout);
    schedule_analysis(shared, events);
    true
}

/// Computes metrics over the current graph and installs them.
pub fn run_analysis(shared: &Shared, events: &dyn Events) {
    let a = {
        let e = shared.read();
        let mut cochange = Vec::new();
        for (path, partners) in &e.history.coupling {
            let Some(a) = e.graph.slot_of_path(path) else {
                continue;
            };
            for p in partners {
                if path.as_str() < p.path.as_str() {
                    if let Some(b) = e.graph.slot_of_path(&p.path) {
                        cochange.push((a, b, p.degree));
                    }
                }
            }
        }
        cochange.sort_by_key(|&(a, b, _)| (a, b));
        analysis::analyse(&e.graph, &cochange)
    };
    let review = {
        let e = shared.read();
        e.review.rebuild(&e.graph, &a)
    };
    let mut e = shared.write();
    e.analysis = Arc::new(a);
    e.review = Arc::new(review);
    let epoch = e.graph.epoch;
    drop(e);
    // Lens colours changed: tiles must be refetched (geometry did not move).
    events.changed(epoch, false);
}

/// Re-reads what the current branch changed (git status plus commits since the base) and
/// installs it. Returns true when the set of changed paths moved; then tiles are refetched.
pub fn run_review(shared: &Shared, events: &dyn Events) -> bool {
    let git = shared.read().git.clone();
    let changes = {
        let mut git = git.lock();
        if !git.opened {
            git.opened = true;
            git.repo = compass_git::Repo::open(&shared.read().root);
        }
        let Some(repo) = &git.repo else { return false };
        match repo.changes() {
            Ok(c) => c,
            Err(err) => {
                eprintln!("branch changes failed: {err}");
                return false;
            }
        }
    };
    let review = {
        let e = shared.read();
        let committed: HashSet<String> = changes.committed.iter().cloned().collect();
        let same = e.review.paths == changes.all()
            && e.review.base == changes.base
            && e.review.committed == committed;
        if same {
            return false;
        }
        review::Review::build(
            &e.graph,
            &e.analysis,
            changes.base_name,
            changes.base,
            changes.committed,
            changes.uncommitted,
        )
    };
    let mut e = shared.write();
    e.review = Arc::new(review);
    let epoch = e.graph.epoch;
    drop(e);
    events.changed(epoch, false);
    true
}

/// Brings git history up to date (everything on the first run, new commits afterwards) and
/// re-clusters with the new co-change weights. Returns false when there is no repository or
/// nothing changed. Mining runs without the engine lock; one run at a time.
pub fn run_history(shared: &Shared, events: &dyn Events) -> bool {
    let t0 = Instant::now();
    let trace = std::env::var_os("COMPASS_TRACE").is_some();
    let git = shared.read().git.clone();
    let mut git = git.lock();
    if !git.opened {
        git.opened = true;
        git.repo = compass_git::Repo::open(&shared.read().root);
    }
    if git.repo.is_none() {
        return false;
    }
    if !git.loaded {
        git.loaded = true;
        let e = shared.read();
        if let Some(store) = &e.store {
            let store = store.lock();
            git.head = store
                .meta("git.head")
                .ok()
                .flatten()
                .filter(|h| !h.is_empty());
            git.commits = store.load_commits().unwrap_or_default();
            if git.commits.is_empty() {
                git.head = None;
            }
        }
    }
    let computed = shared.read().history.head.is_some();
    let mined = {
        let repo = git.repo.as_ref().unwrap();
        match repo.mine(git.head.as_deref(), &|done, total| {
            events.progress(Phase::History, done as u64, total as u64)
        }) {
            Ok(m) => m,
            Err(err) => {
                eprintln!("history mining failed: {err}");
                return false;
            }
        }
    };
    let changed = !mined.commits.is_empty() || !mined.dropped.is_empty() || mined.replace;
    if !changed && computed {
        return false;
    }
    let rows: Vec<compass_core::store::CommitRow> = mined
        .commits
        .into_iter()
        .map(|c| compass_core::store::CommitRow {
            id: c.id,
            time: c.time,
            author: c.author,
            merge: c.merge,
            files: c.files,
        })
        .collect();
    let mine_ms = ms(t0);
    if changed {
        let e = shared.read();
        if let Some(store) = &e.store {
            let mut store = store.lock();
            let r = (|| {
                if mined.replace {
                    store.clear_commits()?;
                } else if !mined.dropped.is_empty() {
                    store.delete_commits(&mined.dropped)?;
                }
                store.write_commits(&rows)?;
                store.set_meta("git.head", mined.head.as_deref().unwrap_or(""))
            })();
            if let Err(err) = r {
                eprintln!("commit store write failed: {err}");
            }
        }
    }
    if mined.replace {
        git.commits.clear();
    } else if !mined.dropped.is_empty() {
        let gone: HashSet<&str> = mined.dropped.iter().map(|s| s.as_str()).collect();
        git.commits.retain(|c| !gone.contains(c.id.as_str()));
    }
    let new = rows.len();
    let mut all = rows;
    all.append(&mut git.commits);
    git.commits = all;
    git.head = mined.head;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut h = {
        let e = shared.read();
        history::History::compute(&git.commits, &|p| e.graph.slot_of_path(p).is_some(), now)
    };
    h.head = git.head.clone();
    drop(git);
    if trace {
        eprintln!(
            "history: {new} new of {} commits, mine {mine_ms:.0} ms, stats {:.0} ms, {} files coupled, total {:.0} ms",
            h.commits,
            h.millis,
            h.coupling.len(),
            ms(t0)
        );
    }
    shared.write().history = Arc::new(h);
    run_analysis(shared, events);
    true
}

/// Benchmark support: forgets the `n` newest mined commits (in memory only) so the next
/// `run_history` mines them again, as if they had just been committed.
#[doc(hidden)]
pub fn forget_newest_commits(shared: &Shared, n: usize) {
    let git = shared.read().git.clone();
    let mut git = git.lock();
    let n = n.min(git.commits.len().saturating_sub(1));
    git.commits.drain(..n);
    git.head = git.commits.first().map(|c| c.id.clone());
}

/// Re-analyses after edits settle (500 ms without further changes), off the request path.
pub fn schedule_analysis(shared: &Shared, events: &dyn Events) {
    let gen = {
        let e = shared.read();
        e.analysis_gen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    };
    let counter = shared.read().analysis_gen.clone();
    let shared = shared.clone();
    let events = events.boxed();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if counter.load(std::sync::atomic::Ordering::SeqCst) == gen {
            run_analysis(&shared, &*events);
            run_review(&shared, &*events);
        }
    });
}

/// One file after reading, hashing and extraction.
struct Processed {
    row: FileRow,
    facts: Option<Arc<FileFacts>>,
    /// Extracted now (not from the cache): to be persisted.
    fresh: bool,
}

/// Whether a language gets facts: Tier 1/2 via tree-sitter, Tier 3 via path mentions.
fn wants_facts(lang: Lang) -> bool {
    compass_parse::langs::spec(lang).is_some()
        || matches!(
            lang,
            Lang::Markdown
                | Lang::Yaml
                | Lang::Json
                | Lang::Toml
                | Lang::Dockerfile
                | Lang::Shell
                | Lang::Html
        )
}

fn extract_facts(lang: Lang, content: &[u8]) -> Option<FileFacts> {
    if compass_parse::langs::spec(lang).is_some() {
        return compass_parse::extract(lang, content);
    }
    let paths = compass_parse::tier3::path_candidates(lang, content);
    if paths.is_empty() {
        return None;
    }
    Some(FileFacts {
        imports: paths
            .into_iter()
            .map(|(p, line)| compass_parse::Import {
                path: format!("{}{p}", resolve::PATH_PREFIX),
                names: Vec::new(),
                line,
            })
            .collect(),
        ..Default::default()
    })
}

/// Reads, hashes and extracts a chunk of files in parallel; cached facts are reused by hash.
fn process_chunk(
    shared: &Shared,
    root: &Path,
    chunk: &[Discovered],
    size_cap: u64,
) -> Vec<Processed> {
    let scanned: Vec<(Discovered, scan::Scanned)> = chunk
        .par_iter()
        .filter_map(|d| {
            let keep = wants_facts(Lang::from_path(&d.path));
            scan::scan_file(root, d, size_cap, keep).map(|s| (d.clone(), s))
        })
        .collect();
    // Cache lookup (one short lock per chunk).
    let cached: Vec<Option<Arc<FileFacts>>> = {
        let e = shared.read();
        match &e.store {
            Some(st) => {
                let st = st.lock();
                scanned
                    .iter()
                    .map(|(_, s)| {
                        s.content
                            .as_ref()
                            .and_then(|_| st.facts(&s.hash).ok().flatten())
                            .and_then(|b| FileFacts::decode(&b))
                            .map(Arc::new)
                    })
                    .collect()
            }
            None => vec![None; scanned.len()],
        }
    };
    let trace = std::env::var_os("COMPASS_TRACE").is_some();
    let slow: Mutex<Vec<(f64, String, u64)>> = Mutex::new(Vec::new());
    let out = scanned
        .into_par_iter()
        .zip(cached)
        .map(|((d, s), cached)| {
            let t = Instant::now();
            let (facts, fresh) = match (cached, &s.content) {
                (Some(f), _) => (Some(f), false),
                (None, Some(c)) => (extract_facts(s.lang, c).map(Arc::new), true),
                (None, None) => (None, false),
            };
            if trace {
                slow.lock().push((ms(t), d.path.clone(), d.size));
            }
            let row = FileRow {
                path: d.path,
                hash: s.hash,
                size: d.size,
                mtime: d.mtime,
                loc: s.loc,
                lang: s.lang,
                flags: s.flags,
            };
            Processed { row, facts, fresh }
        })
        .collect();
    if trace {
        let mut v = slow.into_inner();
        let total: f64 = v.iter().map(|x| x.0).sum();
        v.sort_by(|a, b| b.0.total_cmp(&a.0));
        eprintln!(
            "extract: {:.0} ms cpu over {} files; slowest:",
            total,
            v.len()
        );
        for (t, p, size) in v.iter().take(6) {
            eprintln!("  {t:.0} ms  {size} B  {p}");
        }
    }
    out
}

/// Warm start: facts of unchanged files, decoded in parallel from one pass over the store.
fn load_cached_facts(shared: &Shared) {
    let mut blobs: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let e = shared.read();
        let Some(st) = &e.store else { return };
        let missing: HashSet<&str> = e
            .graph
            .files()
            .filter(|(slot, p)| !e.sem.has_facts(*slot) && wants_facts(Lang::from_path(p)))
            .map(|(_, p)| p)
            .collect();
        let _ = st.lock().for_each_live_fact(|path, data| {
            if missing.contains(path.as_str()) {
                blobs.push((path, data));
            }
        });
    }
    let decoded: Vec<(String, Arc<FileFacts>)> = blobs
        .into_par_iter()
        .filter_map(|(p, b)| FileFacts::decode(&b).map(|f| (p, Arc::new(f))))
        .collect();
    for chunk in decoded.chunks(BATCH) {
        let mut e = shared.write();
        for (p, f) in chunk {
            if let Some(slot) = e.graph.slot_of_path(p) {
                e.apply_facts(slot, Some(f.clone()));
            }
        }
    }
}

/// Resolves files: edges are computed in parallel under the read lock and applied in batches
/// under the write lock, so tile requests keep being served.
fn resolve_in_batches(shared: &Shared, files: &[u32], events: &dyn Events) {
    let total = files.len() as u64;
    let mut done = 0;
    for chunk in files.chunks(BATCH * 2) {
        let results: Vec<resolve::FileEdges> = {
            let e = shared.read();
            chunk
                .par_iter()
                .filter_map(|&f| e.sem.compute(&e.graph, f))
                .collect()
        };
        let mut e = shared.write();
        let Engine { sem, graph, .. } = &mut *e;
        for fe in results {
            sem.apply(graph, fe);
        }
        done += chunk.len() as u64;
        let epoch = e.graph.epoch;
        drop(e);
        events.progress(Phase::Resolve, done, total);
        events.changed(epoch, false);
    }
}

fn set_phase(shared: &Shared, p: Phase) {
    shared.write().phase = p;
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("src/util")).unwrap();
        std::fs::write(
            r.join("src/main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        std::fs::write(r.join("src/util/mod.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(r.join("README.md"), "# hi\n\ntext\n").unwrap();
        dir
    }

    #[test]
    fn cold_then_warm_index() {
        let ws = workspace();
        let db = ws
            .path()
            .join("..")
            .join(format!("compass-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        {
            let e = Engine::new(
                ws.path(),
                Some(Store::open(&db).unwrap()),
                Options::default(),
            )
            .shared();
            run_index(&e, &NoEvents);
            let g = &e.read().graph;
            assert_eq!(g.file_count(), 3);
            let main = g.slot_of_path("src/main.rs").unwrap();
            assert_eq!(g.node(main).loc, 3);
        }
        // Warm: the map is there before discovery runs.
        let e = Engine::new(
            ws.path(),
            Some(Store::open(&db).unwrap()),
            Options::default(),
        )
        .shared();
        let t = run_index(&e, &NoEvents);
        assert!(t.warm_load_ms > 0.0);
        assert_eq!(e.read().graph.file_count(), 3);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn incremental_updates_match_cold_index() {
        let ws = workspace();
        let e = Engine::new(ws.path(), None, Options::default()).shared();
        run_index(&e, &NoEvents);
        std::fs::write(
            ws.path().join("src/main.rs"),
            "fn main() {\n\n    a();\n    b();\n}\n",
        )
        .unwrap();
        std::fs::write(ws.path().join("src/new.rs"), "x\n").unwrap();
        std::fs::remove_file(ws.path().join("README.md")).unwrap();
        // The 50 ms budget is measured by bench/run.mjs (incremental.saveOneFileMs); a wall-clock
        // assertion here would fail whenever the machine is busy.
        assert!(apply_changes(
            &e,
            &[
                "src/main.rs".into(),
                "src/new.rs".into(),
                "README.md".into()
            ],
            &NoEvents
        ));
        let cold = Engine::new(ws.path(), None, Options::default()).shared();
        run_index(&cold, &NoEvents);
        let snapshot = |s: &Shared| {
            let e = s.read();
            let mut v: Vec<(String, u32)> = e
                .graph
                .files()
                .map(|(slot, p)| (p.to_string(), e.graph.node(slot).loc))
                .collect();
            v.sort();
            v
        };
        assert_eq!(snapshot(&e), snapshot(&cold));
    }

    #[test]
    fn data_files_stay_small_unless_configured() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("data")).unwrap();
        std::fs::write(
            r.join("app.py"),
            "def main():
    return 1
"
            .repeat(50),
        )
        .unwrap();
        std::fs::write(
            r.join("data/prices.csv"),
            "a,b,c
"
            .repeat(100_000),
        )
        .unwrap();
        let e = Engine::new(r, None, Options::default()).shared();
        run_index(&e, &NoEvents);
        let area = |e: &Engine, p: &str| {
            let slot = e.graph.slot_of_path(p).unwrap();
            e.with_layout(|l| l.rect_of(&e.graph, slot))
                .map(|r| r.w * r.h)
                .unwrap()
        };
        {
            let e = e.read();
            let csv = e.graph.slot_of_path("data/prices.csv").unwrap();
            assert!(e.graph.node(csv).flags & flags::DATA != 0);
            assert!(
                area(&e, "app.py") > area(&e, "data/prices.csv"),
                "code outweighs a capped CSV"
            );
        }
        e.write().options.cap_data_files = false;
        let e = e.read();
        assert!(
            area(&e, "data/prices.csv") > area(&e, "app.py"),
            "full size when capping is off"
        );
    }

    #[test]
    fn tiles_and_search_work_after_indexing() {
        let ws = workspace();
        let e = Engine::new(ws.path(), None, Options::default()).shared();
        run_index(&e, &NoEvents);
        let e = e.read();
        let buf = e.tile(0, 0, 0);
        assert_eq!(&buf[0..4], b"CMPT");
        let hits = e.search.search("main", 5, |_| 0.0);
        // The function `main` (exact name) ranks first; its file is among the hits too.
        let first = e.graph.node(hits[0].slot);
        assert_eq!((first.kind, &*first.name), (NodeKind::Function, "main"));
        assert!(hits
            .iter()
            .any(|h| e.graph.path(h.slot) == Some("src/main.rs")));
        assert_eq!(
            e.locate("src/main.rs", None),
            e.graph.slot_of_path("src/main.rs")
        );
    }

    /// Runs the system git in `dir` (test setup only; the engine reads history with gix).
    fn git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn history_finds_hidden_coupling_and_updates_incrementally() {
        let ws = workspace();
        let r = ws.path();
        if !git(r, &["init", "-q", "-b", "main"]) {
            return; // git not installed
        }
        git(r, &["config", "user.email", "ann@example.com"]);
        git(r, &["config", "user.name", "Ann"]);
        std::fs::write(r.join("src/schema.sql"), "-- v0\n").unwrap();
        git(r, &["add", "-A"]);
        git(r, &["commit", "-q", "-m", "init"]);
        // main.rs and schema.sql always change together (7 commits with init), with no code edge.
        for i in 1..=6 {
            std::fs::write(
                r.join("src/main.rs"),
                format!("fn main() {{ let v = {i}; }}\n"),
            )
            .unwrap();
            std::fs::write(r.join("src/schema.sql"), format!("-- v{i}\n")).unwrap();
            git(r, &["commit", "-q", "-am", &format!("change {i}")]);
        }
        let db = tempfile::tempdir().unwrap();
        let store = Store::open(&db.path().join("index.db")).unwrap();
        let e = Engine::new(r, Some(store), Options::default()).shared();
        run_index(&e, &NoEvents);
        {
            let e = e.read();
            assert_eq!(e.history.commits, 7);
            let partners = e.partners("src/main.rs");
            assert_eq!(partners.len(), 1);
            let (p, _, linked) = &partners[0];
            assert_eq!(
                (p.path.as_str(), p.shared, *linked),
                ("src/schema.sql", 7, false)
            );
            assert_eq!(e.hidden_pairs(10).len(), 1);
            assert!(e.history.churn_lens("src/main.rs") > 0);
            assert_eq!(e.history.owner_lens("src/main.rs"), 1);
            assert_eq!(
                e.blame("src/main.rs"),
                vec![("ann@example.com".to_string(), 1)]
            );
        }
        // Nothing new: no work.
        assert!(!run_history(&e, &NoEvents));
        // One new commit by someone else is picked up incrementally.
        git(r, &["config", "user.email", "bob@example.com"]);
        std::fs::write(r.join("README.md"), "# changed\n").unwrap();
        git(r, &["commit", "-q", "-am", "docs"]);
        assert!(run_history(&e, &NoEvents));
        assert_eq!(e.read().history.commits, 8);
        // A fresh engine on the same store loads the commits instead of re-mining them.
        drop(e);
        let store = Store::open(&db.path().join("index.db")).unwrap();
        let e2 = Engine::new(r, Some(store), Options::default()).shared();
        run_index(&e2, &NoEvents);
        assert_eq!(e2.read().history.commits, 8);
        assert_eq!(
            e2.read()
                .history
                .owner
                .get("README.md")
                .map(|o| o.0.as_str()),
            Some("ann@example.com")
        );
    }
}
