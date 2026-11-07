//! Golden graph tests: each fixture repo under fixtures/<lang> has an expected.json listing
//! every node and edge in readable form. Any extraction or resolution change shows up as a diff.
//! Regenerate with UPDATE_GOLDEN=1 (and review the diff before committing).

mod common;

use common::dump;
use std::path::{Path, PathBuf};
use terrain_engine::{run_index, Engine, NoEvents, Options};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
}

fn check(lang: &str) {
    let root = fixtures().join(lang);
    let s = Engine::new(&root, None, Options::default()).shared();
    run_index(&s, &NoEvents);
    let (nodes, edges) = dump(&s);
    let actual = serde_json::json!({ "nodes": nodes, "edges": edges });
    let path = root.join("expected.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() || !path.exists() {
        std::fs::write(&path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    // Exclude the golden file itself from the comparison (it is part of the fixture tree).
    if expected != actual {
        let list = |v: &serde_json::Value, k: &str| -> Vec<String> {
            v[k].as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        };
        let mut diff = String::new();
        for k in ["nodes", "edges"] {
            let (a, b) = (list(&expected, k), list(&actual, k));
            for x in a.iter().filter(|x| !b.contains(x)) {
                diff.push_str(&format!("- {k}: {x}\n"));
            }
            for x in b.iter().filter(|x| !a.contains(x)) {
                diff.push_str(&format!("+ {k}: {x}\n"));
            }
        }
        panic!("golden graph for {lang} differs (UPDATE_GOLDEN=1 to accept):\n{diff}");
    }
}

#[test]
fn golden_python() {
    check("python");
}

#[test]
fn golden_typescript() {
    check("typescript");
}

#[test]
fn golden_go() {
    check("go");
}

#[test]
fn golden_rust() {
    check("rust");
}

#[test]
fn golden_java() {
    check("java");
}

#[test]
fn golden_c() {
    check("c");
}

#[test]
fn golden_cpp() {
    check("cpp");
}

#[test]
fn golden_csharp() {
    check("csharp");
}

#[test]
fn golden_kotlin() {
    check("kotlin");
}

#[test]
fn golden_ruby() {
    check("ruby");
}

#[test]
fn golden_php() {
    check("php");
}

#[test]
fn golden_swift() {
    check("swift");
}
