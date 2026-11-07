//! Property test for the incremental path: random sequences of edits applied one by one must
//! yield the same graph as a cold index of the resulting tree.

mod common;

use common::dump;
use std::path::{Path, PathBuf};
use terrain_engine::{apply_changes, run_index, Engine, NoEvents, Options};

/// Small deterministic PRNG (no dependency, reproducible failures).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else if e.file_name() != "expected.json" {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

fn py_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for dir in ["shop", "tests", ""] {
        if let Ok(rd) = std::fs::read_dir(root.join(dir)) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".py") {
                    out.push(if dir.is_empty() {
                        n
                    } else {
                        format!("{dir}/{n}")
                    });
                }
            }
        }
    }
    out.sort();
    out
}

/// One random edit; returns the paths it touched.
fn mutate(rng: &mut Lcg, root: &Path, step: usize) -> Vec<String> {
    let files = py_files(root);
    let f = files[rng.pick(files.len())].clone();
    let path = root.join(&f);
    let models = root.join("shop/models.py");
    // Edits to shared definitions (models.py) are the ones that force other files to
    // re-resolve; they are weighted up so the incremental bookkeeping is really exercised.
    match rng.pick(9) {
        // Add a function that calls an existing one.
        0 => {
            let callee = [
                "money",
                "checkout",
                "apply_discount",
                "total",
                "missing_name",
            ][rng.pick(5)];
            let mut s = std::fs::read_to_string(&path).unwrap_or_default();
            s.push_str(&format!(
                "\n\ndef extra_{step}():\n    return {callee}(1)\n"
            ));
            std::fs::write(&path, s).unwrap();
            vec![f]
        }
        // Rename shared definitions (callers elsewhere lose their target).
        1 | 6 => {
            let Ok(s) = std::fs::read_to_string(&models) else {
                return vec![];
            };
            std::fs::write(
                &models,
                s.replace("def money(", "def money2(")
                    .replace("def total(", "def total2("),
            )
            .unwrap();
            vec!["shop/models.py".into()]
        }
        // Undo the renames (callers elsewhere must find their target again).
        2 | 7 => {
            let Ok(s) = std::fs::read_to_string(&models) else {
                return vec![];
            };
            std::fs::write(
                &models,
                s.replace("def money2(", "def money(")
                    .replace("def total2(", "def total("),
            )
            .unwrap();
            vec!["shop/models.py".into()]
        }
        // Define a name other files already call but that did not exist yet.
        8 => {
            let Ok(mut s) = std::fs::read_to_string(&models) else {
                return vec![];
            };
            if !s.contains("def missing_name(") {
                s.push_str(
                    "

def missing_name(x):
    return x
",
                );
            } else {
                s = s.replace("def missing_name(", "def not_missing(");
            }
            std::fs::write(&models, s).unwrap();
            vec!["shop/models.py".into()]
        }
        // Delete a file (keep models.py and at least a few others).
        3 if files.len() > 4 && f != "shop/models.py" => {
            std::fs::remove_file(&path).unwrap();
            vec![f]
        }
        // Create a file that imports and calls into the package.
        4 | 3 => {
            let name = format!("shop/gen_{step}.py");
            let target = ["models", "pricing", "services", "gen_0"][rng.pick(4)];
            std::fs::write(root.join(&name), format!("from .{target} import Order\n\n\ndef gen_{step}():\n    return Order().total()\n")).unwrap();
            vec![name]
        }
        // Change an import.
        _ => {
            let s = std::fs::read_to_string(&path).unwrap_or_default();
            let changed = if s.contains("from .models import") {
                s.replace("from .models import", "from .pricing import")
            } else {
                format!("from shop.models import money\n{s}")
            };
            std::fs::write(&path, changed).unwrap();
            vec![f]
        }
    }
}

fn run(seed: u64, steps: usize) {
    let tmp = tempfile::tempdir().unwrap();
    let root: PathBuf = tmp.path().join("ws");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/python"),
        &root,
    );
    let live = Engine::new(&root, None, Options::default()).shared();
    run_index(&live, &NoEvents);
    let mut rng = Lcg(seed);
    for step in 0..steps {
        let touched = mutate(&mut rng, &root, step);
        apply_changes(&live, &touched, &NoEvents);
        let cold = Engine::new(&root, None, Options::default()).shared();
        run_index(&cold, &NoEvents);
        let (a, b) = (dump(&live), dump(&cold));
        if a != b {
            let diff = |x: &[String], y: &[String]| {
                x.iter()
                    .filter(|i| !y.contains(i))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            panic!(
                "seed {seed} step {step} touched {touched:?}\nonly incremental: {:#?}\nonly cold: {:#?}",
                [diff(&a.0, &b.0), diff(&a.1, &b.1)].concat(),
                [diff(&b.0, &a.0), diff(&b.1, &a.1)].concat()
            );
        }
    }
}

#[test]
fn incremental_equals_cold_index() {
    for seed in [1, 7, 42, 1234, 99, 2026] {
        run(seed, 30);
    }
}
