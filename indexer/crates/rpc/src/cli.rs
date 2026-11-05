//! Command-line mode for profiling: `compass-indexer index <root> [--store <db>]`.
//! Prints timings as JSON. Run twice with the same store to measure a warm start.

use compass_core::store::Store;
use compass_engine::{run_index, Engine, NoEvents, Options};
use std::path::PathBuf;

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
    println!(
        "{{\"files\":{},\"warmLoadMs\":{:.1},\"firstMapMs\":{:.1},\"discoverMs\":{:.1},\"hashMs\":{:.1},\"totalMs\":{:.1},\"rssMb\":{:.1}}}",
        e.graph.file_count(),
        t.warm_load_ms,
        t.first_map_ms.unwrap_or(0.0),
        t.discover_ms,
        t.hash_ms,
        t.total_ms,
        rss
    );
}
