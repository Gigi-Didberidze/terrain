//! Prints what extraction produces for one file: cargo run --example dump -- <file>
use terrain_core::model::Lang;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let src = std::fs::read(&path).unwrap();
    if std::env::args().any(|a| a == "--tree") {
        let tree =
            terrain_parse::extract::parse(Lang::from_path(&path), &src).expect("parsed language");
        println!("{}", tree.root_node().to_sexp());
        return;
    }
    let f = terrain_parse::extract(Lang::from_path(&path), &src).expect("parsed language");
    for d in &f.defs {
        let parent = d
            .parent
            .map(|p| format!("{}.", f.defs[p as usize].name))
            .unwrap_or_default();
        println!(
            "def  {parent}{} {:?}{}{}",
            d.name,
            d.kind,
            if d.exported { " exported" } else { "" },
            if d.entry { " entry" } else { "" }
        );
    }
    for r in &f.refs {
        let from = r
            .from
            .map(|i| f.defs[i as usize].name.clone())
            .unwrap_or_else(|| "<file>".into());
        println!("ref  {:?} {} (from {from})", r.kind, r.name);
    }
    for i in &f.imports {
        println!("imp  {} {:?}", i.path, i.names);
    }
    println!("ns   {:?}  file-entry {}", f.namespaces, f.entry);
}
