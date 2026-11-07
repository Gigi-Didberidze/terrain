//! Times parsing and each query on one file: cargo run --release --example profile -- <lang> <file>
use compass_core::model::Lang;
use compass_parse::langs::spec;
use std::time::Instant;
use streaming_iterator::StreamingIterator;
use tree_sitter::QueryCursor;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lang = Lang::from_path(&args[1]);
    let src = std::fs::read(&args[1]).unwrap();
    let spec = spec(lang).expect("parsed language");
    let t = Instant::now();
    let tree = compass_parse::extract::parse(lang, &src).unwrap();
    println!(
        "parse   {:>7.1} ms  ({} bytes)",
        t.elapsed().as_secs_f64() * 1e3,
        src.len()
    );
    let queries = [
        ("defs", Some(&spec.defs)),
        ("refs", Some(&spec.refs)),
        ("imports", spec.imports.as_ref()),
        ("entries", spec.entries.as_ref()),
    ];
    for (name, q) in queries {
        let Some(q) = q else { continue };
        let t = Instant::now();
        let mut c = QueryCursor::new();
        let mut m = c.matches(q, tree.root_node(), src.as_slice());
        let mut n = 0;
        while m.next().is_some() {
            n += 1;
        }
        println!(
            "{name:<8}{:>7.1} ms  ({n} matches, {} patterns)",
            t.elapsed().as_secs_f64() * 1e3,
            q.pattern_count()
        );
    }
    let t = Instant::now();
    let f = compass_parse::extract(lang, &src).unwrap();
    println!(
        "extract {:>7.1} ms  ({} defs, {} refs)",
        t.elapsed().as_secs_f64() * 1e3,
        f.defs.len(),
        f.refs.len()
    );
}
