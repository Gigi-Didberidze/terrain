//! compass-engine: the indexing pipeline (discover → hash → parse → resolve → analyse → persist)
//! and the queries every view runs against the graph.

pub mod scan;
pub mod search;

use compass_core::layout::Layout;
use compass_core::model::*;
use compass_core::store::{FileRow, Store};
use compass_core::tiles::{self, LensValues};
use compass_core::Graph;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use scan::{Discovered, DEFAULT_SIZE_CAP};
use search::SearchIndex;
use std::collections::{HashMap, HashSet};
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
}

pub struct NoEvents;
impl Events for NoEvents {
    fn progress(&self, _: Phase, _: u64, _: u64) {}
    fn changed(&self, _: u64, _: bool) {}
}

#[derive(Clone, Debug)]
pub struct Options {
    pub size_cap: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            size_cap: DEFAULT_SIZE_CAP,
        }
    }
}

pub struct Engine {
    pub root: PathBuf,
    pub options: Options,
    pub graph: Graph,
    pub search: SearchIndex,
    /// Last known state of every file, mirrored in the store.
    pub files: HashMap<String, FileRow>,
    layout: Mutex<Option<Layout>>,
    store: Option<Mutex<Store>>,
    pub phase: Phase,
    pub timings: Timings,
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
            files: HashMap::new(),
            layout: Mutex::new(None),
            store: store.map(Mutex::new),
            phase: Phase::Idle,
            timings: Timings::default(),
        }
    }

    pub fn shared(self) -> Shared {
        Arc::new(RwLock::new(self))
    }

    fn put_file(&mut self, row: FileRow) {
        let slot = self
            .graph
            .upsert_file(&row.path, row.loc, row.lang, row.flags);
        let name = row.path.rsplit('/').next().unwrap_or(&row.path).to_string();
        self.search.upsert(slot, &name, &row.path);
        self.files.insert(row.path.clone(), row);
    }

    fn drop_file(&mut self, path: &str) {
        if let Some(slot) = self.graph.slot_of_path(path) {
            for s in self.graph.file_symbols(slot) {
                self.search.remove(s);
            }
            self.search.remove(slot);
        }
        self.graph.remove_file(path);
        self.files.remove(path);
    }

    fn persist(&self, rows: &[FileRow], deleted: &[String]) {
        if let Some(store) = &self.store {
            if let Err(e) = store.lock().write_batch(rows, &[], deleted) {
                eprintln!("store write failed: {e}");
            }
        }
    }

    /// Runs `f` with an up-to-date layout.
    pub fn with_layout<R>(&self, f: impl FnOnce(&mut Layout) -> R) -> R {
        let mut guard = self.layout.lock();
        if guard
            .as_ref()
            .is_none_or(|l| l.epoch != self.graph.layout_epoch)
        {
            *guard = Some(Layout::compute(&self.graph));
        }
        f(guard.as_mut().unwrap())
    }

    /// One packed tile.
    pub fn tile(&self, z: u32, x: u32, y: u32) -> Vec<u8> {
        self.with_layout(|layout| {
            let items = tiles::query(&self.graph, layout, z, x, y, 50_000);
            tiles::pack(&self.graph, layout, z, x, y, &items, |_| {
                LensValues::default()
            })
        })
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
}

/// Full index run: warm start from the store, then bring it up to date with the workspace.
/// Returns the timings. Heavy work runs outside the write lock; results are merged in batches.
pub fn run_index(shared: &Shared, events: &dyn Events) -> Timings {
    let t0 = Instant::now();
    let (root, size_cap) = {
        let e = shared.read();
        (e.root.clone(), e.options.size_cap)
    };

    // Warm start: everything we knew last time is on the map before we touch the disk.
    let rows = {
        let e = shared.read();
        e.store
            .as_ref()
            .and_then(|s| s.lock().load_files().ok())
            .unwrap_or_default()
    };
    let warm = !rows.is_empty();
    if warm {
        let mut e = shared.write();
        for r in rows {
            e.put_file(r);
        }
        e.timings.warm_load_ms = ms(t0);
        e.timings.first_map_ms = Some(ms(t0));
        let epoch = e.graph.epoch;
        drop(e);
        events.changed(epoch, true);
    }

    // Discover.
    set_phase(shared, Phase::Discover);
    let mut seen: HashSet<String> = HashSet::new();
    let mut to_scan: Vec<Discovered> = Vec::new();
    let mut last_emit = Instant::now();
    let discovered = scan::discover(&root, |batch| {
        let mut e = shared.write();
        for d in batch {
            let known = e.files.get(&d.path);
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
                e.put_file(row);
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
            .files
            .keys()
            .filter(|p| !seen.contains(*p))
            .cloned()
            .collect();
        for p in &gone {
            e.drop_file(p);
        }
        e.persist(&[], &gone);
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
        let scanned: Vec<(Discovered, scan::Scanned)> = chunk
            .par_iter()
            .filter_map(|d| scan::scan_file(&root, d, size_cap, false).map(|s| (d.clone(), s)))
            .collect();
        let mut e = shared.write();
        let mut rows = Vec::with_capacity(scanned.len());
        for (d, s) in scanned {
            let row = FileRow {
                path: d.path,
                hash: s.hash,
                size: d.size,
                mtime: d.mtime,
                loc: s.loc,
                lang: s.lang,
                flags: s.flags,
            };
            rows.push(row.clone());
            e.put_file(row);
        }
        e.persist(&rows, &[]);
        done += chunk.len() as u64;
        let epoch = e.graph.epoch;
        drop(e);
        events.progress(Phase::Hash, done, total);
        events.changed(epoch, true);
    }

    let mut e = shared.write();
    e.timings.hash_ms = ms(t0);
    e.timings.total_ms = ms(t0);
    e.phase = Phase::Idle;
    let _ = discovered;
    let t = e.timings.clone();
    drop(e);
    events.progress(Phase::Idle, total, total);
    t
}

/// Incremental update for paths reported by the editor or the file watcher (saves, creates,
/// deletes, branch switches). Returns true if the map changed.
pub fn apply_changes(shared: &Shared, paths: &[String], events: &dyn Events) -> bool {
    let (root, size_cap) = {
        let e = shared.read();
        (e.root.clone(), e.options.size_cap)
    };
    let work: Vec<(String, Option<(Discovered, scan::Scanned)>)> = paths
        .par_iter()
        .map(|p| {
            let full = root.join(p);
            match std::fs::metadata(&full) {
                Ok(m) if m.is_file() => {
                    let d = Discovered {
                        path: p.clone(),
                        size: m.len(),
                        mtime: scan::mtime_ns(&m),
                    };
                    let s = scan::scan_file(&root, &d, size_cap, false);
                    (p.clone(), s.map(|s| (d, s)))
                }
                _ => (p.clone(), None),
            }
        })
        .collect();
    let mut e = shared.write();
    let before = e.graph.layout_epoch;
    let mut rows = Vec::new();
    let mut gone = Vec::new();
    for (p, r) in work {
        match r {
            Some((d, s)) => {
                if e.files.get(&p).is_some_and(|old| old.hash == s.hash) {
                    continue;
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
                rows.push(row.clone());
                e.put_file(row);
            }
            None => {
                if e.files.contains_key(&p) {
                    e.drop_file(&p);
                    gone.push(p);
                }
            }
        }
    }
    if rows.is_empty() && gone.is_empty() {
        return false;
    }
    e.persist(&rows, &gone);
    let epoch = e.graph.epoch;
    let layout = e.graph.layout_epoch != before;
    drop(e);
    events.changed(epoch, layout);
    true
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
        let t = Instant::now();
        assert!(apply_changes(
            &e,
            &[
                "src/main.rs".into(),
                "src/new.rs".into(),
                "README.md".into()
            ],
            &NoEvents
        ));
        assert!(t.elapsed().as_millis() < 50);
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
    fn tiles_and_search_work_after_indexing() {
        let ws = workspace();
        let e = Engine::new(ws.path(), None, Options::default()).shared();
        run_index(&e, &NoEvents);
        let e = e.read();
        let buf = e.tile(0, 0, 0);
        assert_eq!(&buf[0..4], b"CMPT");
        let hits = e.search.search("main", 5, |_| 0.0);
        assert_eq!(e.graph.path(hits[0].slot), Some("src/main.rs"));
        assert_eq!(
            e.locate("src/main.rs", None),
            e.graph.slot_of_path("src/main.rs")
        );
    }
}
