//! Command-line mode for profiling: `terrain-indexer index <root> [--store <db>]`.
//! Prints timings and graph counts as JSON. Run twice with the same store to measure a warm start.

use compass_core::model::{EdgeKind, NodeKind};
use compass_core::store::Store;
use compass_engine::{run_index, Engine, NoEvents, Options};
use std::path::PathBuf;

/// `terrain-indexer imports <root> [--lang python|typescript|...]`: one JSON line per import
/// with the files Compass resolves it to, for comparison against language tooling.
pub fn imports(args: &[String]) {
    let Some(root) = args.first() else {
        eprintln!("usage: terrain-indexer imports <root> [--lang <lang>]");
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
        eprintln!("usage: terrain-indexer index <root> [--store <db>]");
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

/// `terrain-indexer analyse <root>`: load-bearing ranking, communities, cycles, dead code.
pub fn analyse(args: &[String]) {
    let Some(root) = args.first() else {
        eprintln!("usage: terrain-indexer analyse <root>");
        std::process::exit(2);
    };
    let shared = Engine::new(&PathBuf::from(root), None, Options::default()).shared();
    run_index(&shared, &NoEvents);
    let e = shared.read();
    let a = e.analysis.clone();
    let path = |s: u32| e.graph.path(s).unwrap_or("?").to_string();
    println!("analysis: {:.0} ms, {} files", a.millis, a.files.len());
    let mut ranked: Vec<u32> = a.files.clone();
    ranked.sort_by(|x, y| a.rank_of(*y).total_cmp(&a.rank_of(*x)));
    println!("\ntop 20 load-bearing files (dependents within 6 steps):");
    for (i, &f) in ranked.iter().take(20).enumerate() {
        println!(
            "{:>3}. {:<60} rank {:.5}  blast {}",
            i + 1,
            path(f),
            a.rank_of(f),
            a.file_blast(f, 6, 1_000_000).len()
        );
    }
    let mut comms: Vec<(usize, u32)> = a
        .community_sizes
        .iter()
        .enumerate()
        .map(|(c, &n)| (c, n))
        .collect();
    comms.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("\n{} communities; largest:", comms.len());
    for &(c, size) in comms.iter().take(8) {
        let members: Vec<String> = a
            .files
            .iter()
            .enumerate()
            .filter(|(i, _)| a.community[*i] as usize == c)
            .take(4)
            .map(|(_, &f)| path(f))
            .collect();
        println!("  {size:>5} files  e.g. {}", members.join(", "));
    }
    println!(
        "\n{} file cycles, {} directory cycles; largest file cycles:",
        a.file_cycles.len(),
        a.dir_cycles.len()
    );
    for c in a.file_cycles.iter().take(6) {
        let brk = c
            .break_edge
            .map(|(x, y)| format!("{} -> {}", path(x), path(y)))
            .unwrap_or_default();
        println!(
            "  {} files, e.g. {}; break: {brk}",
            c.nodes.len(),
            path(c.nodes[0])
        );
    }
    println!("\n{} dead-code candidates", a.dead.len());
}

/// `terrain-indexer cycles <root>`: every import cycle as one JSON line (member paths).
pub fn cycles(args: &[String]) {
    let Some(root) = args.first() else {
        eprintln!("usage: terrain-indexer cycles <root>");
        std::process::exit(2);
    };
    let shared = Engine::new(&PathBuf::from(root), None, Options::default()).shared();
    run_index(&shared, &NoEvents);
    let e = shared.read();
    for c in &e.analysis.file_cycles {
        let members: Vec<&str> = c
            .nodes
            .iter()
            .map(|&s| e.graph.path(s).unwrap_or("?"))
            .collect();
        println!("{}", serde_json::json!({ "files": members }));
    }
}

/// `terrain-indexer history <root> [--store <db>] [--rewind <n>]`: mines git history (index
/// first, so files are known) and prints timings; with `--rewind`, then forgets the n newest
/// commits and mines them again to time an incremental update.
pub fn history(args: &[String]) {
    let Some(root) = args.first() else {
        eprintln!("usage: terrain-indexer history <root> [--store <db>] [--rewind <n>]");
        std::process::exit(2);
    };
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let store = flag("--store").map(|p| Store::open(&PathBuf::from(p)).expect("open store"));
    let rewind: usize = flag("--rewind").and_then(|n| n.parse().ok()).unwrap_or(0);
    let shared = Engine::new(&PathBuf::from(root), store, Options::default()).shared();
    let t0 = std::time::Instant::now();
    let t = run_index(&shared, &NoEvents);
    let total = t0.elapsed().as_secs_f64() * 1000.0;
    let (commits, coupled, hidden, misplaced, stats_ms) = {
        let e = shared.read();
        (
            e.history.commits,
            e.history.coupling.len(),
            e.hidden_pairs(usize::MAX).len(),
            e.analysis.misplaced.len(),
            e.history.millis,
        )
    };
    let mut out = serde_json::json!({
        "commits": commits,
        "coupledFiles": coupled,
        "hiddenPairs": hidden,
        "misplaced": misplaced,
        "statsMs": stats_ms,
        "indexMs": t.total_ms,
        "totalMs": total,
    });
    if args.iter().any(|a| a == "--list") {
        let e = shared.read();
        let path = |s: u32| e.graph.path(s).unwrap_or("?").to_string();
        eprintln!("top authors: {:?}", e.history.top_authors);
        eprintln!("hidden dependencies (strongest 15):");
        for (a, b, p) in e.hidden_pairs(15) {
            eprintln!(
                "  {:.2} {:>4}  {}  <->  {}",
                p.degree,
                p.shared,
                path(a),
                path(b)
            );
        }
        eprintln!("misplaced (first 15):");
        for m in e.analysis.misplaced.iter().take(15) {
            eprintln!(
                "  {}  -> module {:.0}% in {}/",
                path(m.file),
                m.share * 100.0,
                path(m.home)
            );
        }
        let mut churn: Vec<(&String, &u32)> = e.history.churn.iter().collect();
        churn.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        eprintln!("most churn (90 days): {:?}", &churn[..churn.len().min(8)]);
    }
    if rewind > 0 {
        compass_engine::forget_newest_commits(&shared, rewind);
        let t1 = std::time::Instant::now();
        let changed = compass_engine::run_history(&shared, &NoEvents);
        out["incremental"] = serde_json::json!({
            "commits": rewind,
            "changed": changed,
            "ms": t1.elapsed().as_secs_f64() * 1000.0,
        });
    }
    println!("{out}");
}
