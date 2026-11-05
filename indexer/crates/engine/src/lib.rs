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
    /// Last known content state of every file, indexed by slot; mirrored in the store.
    meta: Vec<FileMeta>,
    layout: Mutex<Option<Layout>>,
    store: Option<Mutex<Store>>,
    pub phase: Phase,
    pub timings: Timings,
    /// Set once any tile has been served (lets the warm start prioritise the first view).
    tiles_served: Arc<std::sync::atomic::AtomicBool>,
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
        }
    }

    pub fn shared(self) -> Shared {
        Arc::new(RwLock::new(self))
    }

    /// Adds or updates a file. `index` adds it to search right away; the warm start skips that
    /// and indexes everything in one parallel pass after the map is up.
    fn put_file(&mut self, row: &FileRow, index: bool) {
        let slot = self
            .graph
            .upsert_file(&row.path, row.loc, row.lang, row.flags) as usize;
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

    fn drop_file(&mut self, path: &str) {
        if let Some(slot) = self.graph.slot_of_path(path) {
            for s in self.graph.file_symbols(slot) {
                self.search.remove(s);
            }
            self.search.remove(slot);
        }
        self.graph.remove_file(path);
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
        self.tiles_served
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.with_layout(|layout| {
            let items = tiles::query(&self.graph, layout, z, x, y, tiles::TILE_ITEM_LIMIT);
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
        let tc = Instant::now();
        let scanned: Vec<(Discovered, scan::Scanned)> = chunk
            .par_iter()
            .filter_map(|d| scan::scan_file(&root, d, size_cap, false).map(|s| (d.clone(), s)))
            .collect();
        let scan_ms = ms(tc);
        let tm = Instant::now();
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
            e.put_file(&row, true);
        }
        let merge_ms = ms(tm);
        let tp = Instant::now();
        e.persist(&rows, &[]);
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

    let mut e = shared.write();
    e.timings.hash_ms = ms(t0);
    e.timings.total_ms = ms(t0);
    e.phase = Phase::Idle;
    e.save_snapshot();
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
                if e.meta_of(&p).is_some_and(|old| old.hash == s.hash) {
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
                e.put_file(&row, true);
            }
            None => {
                if e.meta_of(&p).is_some() {
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
