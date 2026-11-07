//! Command-line mode for profiling: `compass-indexer index <root> [--store <db>]`.
//! Prints timings and graph counts as JSON. Run twice with the same store to measure a warm start.

use compass_core::model::{EdgeKind, NodeKind};
use compass_core::store::Store;
use compass_engine::{run_index, Engine, NoEvents, Options};
use std::path::PathBuf;

/// `compass-indexer imports <root> [--lang python|typescript|...]`: one JSON line per import
/// with the files Compass resolves it to, for comparison against language tooling.
pub fn imports(args: &[String]) {
    let Some(root) = args.first() else {
        eprintln!("usage: compass-indexer imports <root> [--lang <lang>]");
        std::process::exit(2);
    };
    let only = args
        .iter()
        .position(|a| a == "--lang")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let shared = Engine::new(&PathBuf::from(root), None, Options::default()).shared();
    run_index(&shared, &NoEvents);
    let e = shared.read();
    let mut files: Vec<(u32, String)> = e.graph.files().map(|(s, p)| (s, p.to_string())).collect();
    files.sort_by(|a, b| a.1.cmp(&b.1));
    for (slot, path) in files {
        let lang = e.graph.node(slot).lang.as_str();
        if only
            .as_deref()
            .is_some_and(|l| l != lang && !(l == "typescript" && lang == "tsx"))
        {
            continue;
        }
        for (imp, targets) in e.sem.import_targets(&e.graph, slot) {
            let line = serde_json::json!({ "from": path, "lang": lang, "spec": imp.path, "names": imp.names, "line": imp.line, "ours": targets });
            println!("{line}");
        }
    }
}

pub fn index(args: &[String]) {
    let Some(root) = args.first() else {
        eprintln!("usage: compass-indexer index <root> [--store <db>]");
        std::process::exit(2);
    };
    let store = args
        .iter()
        .position(|a| a == "--store")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let store = store.map(|p| Store::open(&p).expect("open store"));
    let shared = Engine::new(&PathBuf::from(root), store, Options::default()).shared();
    let t = run_index(&shared, &NoEvents);
    let e = shared.read();
    let rss = memory_stats::memory_stats()
        .map(|m| m.physical_mem as f64 / 1048576.0)
        .unwrap_or(0.0);
    let mut symbols = 0usize;
    let mut edges = [0usize; 7];
    for (slot, n) in e.graph.nodes.iter().enumerate() {
        if !n.alive() {
            continue;
        }
        if n.kind != NodeKind::Dir && n.kind != NodeKind::File {
            symbols += 1;
        }
        for ed in e.graph.out_edges(slot as u32) {
            edges[ed.kind as usize] += 1;
        }
    }
    let unresolved: u32 = e.sem.unresolved.values().sum();
    let resolved_imports: usize = e.sem.imports_of.values().map(|v| v.len()).sum();
    println!(
        "{{\"files\":{},\"symbols\":{symbols},\"imports\":{},\"calls\":{},\"references\":{},\"inherits\":{},\"implements\":{},\"importTargets\":{resolved_imports},\"unresolvedImportSpecs\":{unresolved},\"warmLoadMs\":{:.1},\"firstMapMs\":{:.1},\"discoverMs\":{:.1},\"hashMs\":{:.1},\"totalMs\":{:.1},\"rssMb\":{:.1}}}",
        e.graph.file_count(),
        edges[EdgeKind::Imports as usize],
        edges[EdgeKind::Calls as usize],
        edges[EdgeKind::References as usize],
        edges[EdgeKind::Inherits as usize],
        edges[EdgeKind::Implements as usize],
        t.warm_load_ms,
        t.first_map_ms.unwrap_or(0.0),
        t.discover_ms,
        t.hash_ms,
        t.total_ms,
        rss
    );
}
