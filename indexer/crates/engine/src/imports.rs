//! Import resolution: turns import specifiers into workspace files, per language. Specifiers
//! that resolve outside the workspace (standard library, node_modules, crates.io) resolve to
//! nothing and are counted as external.

use crate::project::{join, Project};
use compass_core::model::Lang;
use compass_parse::Import;
use std::collections::{HashMap, HashSet};

/// Workspace file lookups, maintained incrementally by the engine.
#[derive(Default, Debug)]
pub struct FileIndex {
    files: HashSet<String>,
    by_name: HashMap<String, Vec<String>>,
    by_dir: HashMap<String, Vec<String>>,
    /// Last segment of a directory → directory paths.
    dir_names: HashMap<String, Vec<String>>,
}

fn dir_of(p: &str) -> &str {
    p.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}
fn name_of(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

impl FileIndex {
    pub fn add(&mut self, path: &str) {
        if !self.files.insert(path.to_string()) {
            return;
        }
        self.by_name
            .entry(name_of(path).to_string())
            .or_default()
            .push(path.to_string());
        let dir = dir_of(path).to_string();
        let list = self.by_dir.entry(dir.clone()).or_default();
        if list.is_empty() {
            self.dir_names
                .entry(name_of(&dir).to_string())
                .or_default()
                .push(dir.clone());
        }
        list.push(path.to_string());
    }

    pub fn remove(&mut self, path: &str) {
        if !self.files.remove(path) {
            return;
        }
        if let Some(v) = self.by_name.get_mut(name_of(path)) {
            v.retain(|p| p != path);
        }
        let dir = dir_of(path);
        if let Some(v) = self.by_dir.get_mut(dir) {
            v.retain(|p| p != path);
            if v.is_empty() {
                self.by_dir.remove(dir);
                if let Some(d) = self.dir_names.get_mut(name_of(dir)) {
                    d.retain(|x| x != dir);
                }
            }
        }
    }

    pub fn exists(&self, path: &str) -> bool {
        self.files.contains(path)
    }

    pub fn files_in(&self, dir: &str) -> &[String] {
        self.by_dir.get(dir).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Files whose path ends with `suffix` at a segment boundary.
    pub fn with_suffix(&self, suffix: &str) -> Vec<&str> {
        let name = name_of(suffix);
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .filter(|p| {
                p.as_str() == suffix
                    || (p.len() > suffix.len()
                        && p.ends_with(suffix)
                        && p.as_bytes()[p.len() - suffix.len() - 1] == b'/')
            })
            .map(|s| s.as_str())
            .collect()
    }

    /// Directories whose path ends with `suffix` at a segment boundary.
    pub fn dirs_with_suffix(&self, suffix: &str) -> Vec<&str> {
        let name = name_of(suffix);
        self.dir_names
            .get(name)
            .into_iter()
            .flatten()
            .filter(|d| {
                d.as_str() == suffix
                    || (d.ends_with(suffix) && d.as_bytes()[d.len() - suffix.len() - 1] == b'/')
            })
            .map(|s| s.as_str())
            .collect()
    }
}

/// Picks the candidate closest to `from` (longest shared directory prefix, then shortest path).
fn closest<'a>(from: &str, mut cands: Vec<&'a str>) -> Option<&'a str> {
    let shared = |p: &str| {
        from.split('/')
            .zip(p.split('/'))
            .take_while(|(a, b)| a == b)
            .count()
    };
    cands.sort_by(|a, b| {
        shared(b)
            .cmp(&shared(a))
            .then(a.len().cmp(&b.len()))
            .then(a.cmp(b))
    });
    cands.first().copied()
}

pub struct Ctx<'a> {
    pub idx: &'a FileIndex,
    pub project: &'a Project,
    /// Namespace → files declaring it (C#, Java/Kotlin packages, PHP).
    pub namespaces: &'a HashMap<String, Vec<String>>,
}

/// Resolves one import of `from` to workspace files.
pub fn resolve(lang: Lang, from: &str, imp: &Import, ctx: &Ctx) -> Vec<String> {
    let s = imp.path.as_str();
    let out = match lang {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => js(from, s, ctx),
        Lang::Python => python(from, imp, ctx),
        Lang::Go => go(s, ctx),
        Lang::Rust => rust(from, s, ctx),
        Lang::Java | Lang::Kotlin => jvm(lang, s, ctx),
        Lang::C | Lang::Cpp => c(from, s, ctx),
        Lang::CSharp => namespace_files(
            s.trim_start_matches("static ")
                .rsplit('=')
                .next()
                .unwrap_or(s)
                .trim(),
            ctx,
            50,
        ),
        Lang::Ruby => ruby(from, s, ctx),
        Lang::Php => php(from, s, ctx),
        Lang::Swift => swift(s, ctx),
        Lang::Lua => lua(s, ctx),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = out.into_iter().filter(|t| t != from).collect();
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------- JavaScript / TypeScript

const JS_EXT: [&str; 9] = [
    ".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts",
];

fn js_file(base: &str, idx: &FileIndex) -> Option<String> {
    if idx.exists(base) && !base.ends_with('/') {
        // A literal .js specifier usually means the .ts source (ESM-style TypeScript).
        if let Some(stem) = base
            .strip_suffix(".js")
            .or_else(|| base.strip_suffix(".mjs"))
            .or_else(|| base.strip_suffix(".cjs"))
        {
            for e in [".ts", ".tsx", ".mts", ".cts"] {
                let t = format!("{stem}{e}");
                if idx.exists(&t) {
                    return Some(t);
                }
            }
        }
        return Some(base.to_string());
    }
    if let Some(stem) = base
        .strip_suffix(".js")
        .or_else(|| base.strip_suffix(".jsx"))
        .or_else(|| base.strip_suffix(".mjs"))
        .or_else(|| base.strip_suffix(".cjs"))
    {
        for e in [".ts", ".tsx", ".mts", ".cts", ".d.ts"] {
            let t = format!("{stem}{e}");
            if idx.exists(&t) {
                return Some(t);
            }
        }
    }
    for e in JS_EXT.iter().chain([".json", ".vue", ".svelte"].iter()) {
        let t = format!("{base}{e}");
        if idx.exists(&t) {
            return Some(t);
        }
    }
    for e in JS_EXT {
        let t = format!("{base}/index{e}");
        if idx.exists(&t) {
            return Some(t);
        }
    }
    None
}

fn js(from: &str, s: &str, ctx: &Ctx) -> Vec<String> {
    let idx = ctx.idx;
    if s.starts_with("./") || s.starts_with("../") || s == "." || s == ".." {
        return js_file(&join(dir_of(from), s), idx).into_iter().collect();
    }
    if s.starts_with('/') {
        return js_file(s.trim_start_matches('/'), idx)
            .into_iter()
            .collect();
    }
    if let Some(cfg) = ctx.project.tsconfig_for(from) {
        for (pattern, targets) in &cfg.paths {
            let star = if let Some(prefix) = pattern.strip_suffix('*') {
                match s.strip_prefix(prefix) {
                    Some(rest) => Some(rest),
                    None => continue,
                }
            } else if pattern == s {
                None
            } else {
                continue;
            };
            for t in targets {
                let cand = match star {
                    Some(rest) => t.replacen('*', rest, 1),
                    None => t.clone(),
                };
                if let Some(f) = js_file(&cand, idx) {
                    return vec![f];
                }
            }
        }
        if let Some(base) = &cfg.base_url {
            if let Some(f) = js_file(&join(base, s), idx) {
                return vec![f];
            }
        }
    }
    // Workspace packages (monorepos): "@scope/pkg" or "@scope/pkg/sub/path".
    let mut segs = s.splitn(3, '/');
    let first = segs.next().unwrap_or("");
    let pkg = if first.starts_with('@') {
        format!("{first}/{}", segs.next().unwrap_or(""))
    } else {
        first.to_string()
    };
    if let Some((dir, entry)) = ctx.project.packages.get(&pkg) {
        let rest = s[pkg.len()..].trim_start_matches('/');
        if rest.is_empty() {
            let mut cands = Vec::new();
            if let Some(e) = entry {
                cands.push(join(dir, e));
            }
            cands.push(join(dir, "src/index"));
            cands.push(join(dir, "index"));
            for c in cands {
                if let Some(f) = js_file(&c, idx) {
                    return vec![f];
                }
            }
        } else {
            for c in [join(dir, rest), join(dir, &format!("src/{rest}"))] {
                if let Some(f) = js_file(&c, idx) {
                    return vec![f];
                }
            }
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------- Python

fn py_module(dotted: &str, from: &str, idx: &FileIndex) -> Option<String> {
    let rel = dotted.replace('.', "/");
    let mut cands = Vec::new();
    for suffix in [
        format!("{rel}.py"),
        format!("{rel}.pyi"),
        format!("{rel}/__init__.py"),
    ] {
        for c in idx.with_suffix(&suffix) {
            // Python finds a top-level module only in a source root: a directory that is not
            // itself a package. Without this, `import json` would match some_pkg/json.py.
            let root = &c[..c.len() - suffix.len()];
            let root = root.trim_end_matches('/');
            let init = if root.is_empty() {
                "__init__.py".to_string()
            } else {
                format!("{root}/__init__.py")
            };
            if !idx.exists(&init) {
                cands.push(c);
            }
        }
    }
    closest(from, cands).map(String::from)
}

fn python(from: &str, imp: &Import, ctx: &Ctx) -> Vec<String> {
    let s = imp.path.as_str();
    let idx = ctx.idx;
    let dots = s.chars().take_while(|&c| c == '.').count();
    if dots > 0 {
        // Relative: one dot = this package.
        let mut base = dir_of(from).to_string();
        for _ in 1..dots {
            base = dir_of(&base).to_string();
        }
        let rest = s[dots..].replace('.', "/");
        let module_base = if rest.is_empty() {
            base.clone()
        } else {
            join(&base, &rest)
        };
        let module = |p: &str| -> Option<String> {
            [
                format!("{p}.py"),
                format!("{p}/__init__.py"),
                format!("{p}.pyi"),
            ]
            .into_iter()
            .find(|c| idx.exists(c))
        };
        let mut out = Vec::new();
        // `from . import a, b`: each name may be a submodule.
        for n in &imp.names {
            if let Some(m) = module(&join(&module_base, n)) {
                out.push(m);
            }
        }
        if out.len() < imp.names.len() || imp.names.is_empty() {
            if let Some(m) = module(&module_base) {
                out.push(m);
            }
        }
        return out;
    }
    let mut out = Vec::new();
    for n in &imp.names {
        if let Some(m) = py_module(&format!("{s}.{n}"), from, idx) {
            out.push(m);
        }
    }
    if out.len() < imp.names.len() || imp.names.is_empty() {
        if let Some(m) = py_module(s, from, idx) {
            out.push(m);
        }
    }
    out
}

// ---------------------------------------------------------------- Go

fn go(s: &str, ctx: &Ctx) -> Vec<String> {
    let dir = ctx
        .project
        .go_modules
        .iter()
        .find_map(|(m, d)| {
            if s == m {
                Some(d.clone())
            } else {
                s.strip_prefix(&format!("{m}/")).map(|rest| join(d, rest))
            }
        })
        .or_else(|| {
            // Vendored dependency.
            ctx.idx
                .dirs_with_suffix(&format!("vendor/{s}"))
                .first()
                .map(|d| d.to_string())
        });
    let Some(dir) = dir else { return Vec::new() };
    ctx.idx
        .files_in(&dir)
        .iter()
        .filter(|f| f.ends_with(".go") && !f.ends_with("_test.go"))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------- Rust

fn rust_mod_dir(file: &str) -> String {
    let name = name_of(file);
    let dir = dir_of(file);
    if matches!(name, "lib.rs" | "main.rs" | "mod.rs") {
        dir.to_string()
    } else {
        join(dir, name.trim_end_matches(".rs"))
    }
}

fn rust_module_file(dir: &str, seg: &str, idx: &FileIndex) -> Option<String> {
    [
        join(dir, &format!("{seg}.rs")),
        join(dir, &format!("{seg}/mod.rs")),
    ]
    .into_iter()
    .find(|p| idx.exists(p))
}

/// Expands `a::{b, c::d}` into `a::b`, `a::c::d` (one level of braces is enough in practice).
fn expand_use(s: &str) -> Vec<String> {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.split(" as ").next().unwrap_or(&s).to_string();
    if let (Some(open), true) = (s.find('{'), s.ends_with('}')) {
        let prefix = s[..open].trim_end_matches("::");
        let inner = &s[open + 1..s.len() - 1];
        let mut depth = 0;
        let mut parts = Vec::new();
        let mut cur = String::new();
        for ch in inner.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
            cur.push(ch);
        }
        parts.push(cur);
        return parts
            .into_iter()
            .map(|p| {
                p.trim()
                    .split(" as ")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            })
            .filter(|p| !p.is_empty())
            .flat_map(|p| {
                if p == "self" {
                    vec![prefix.to_string()]
                } else {
                    expand_use(&format!("{prefix}::{p}"))
                }
            })
            .collect();
    }
    vec![s]
}

fn rust(from: &str, s: &str, ctx: &Ctx) -> Vec<String> {
    let idx = ctx.idx;
    if let Some(m) = s.strip_prefix("mod:") {
        return rust_module_file(&rust_mod_dir(from), m, idx)
            .into_iter()
            .collect();
    }
    let mut out = Vec::new();
    for path in expand_use(s) {
        let segs: Vec<&str> = path.split("::").filter(|x| !x.is_empty()).collect();
        let Some(&first) = segs.first() else { continue };
        let (mut file, mut rest): (String, &[&str]) = match first {
            "crate" => match ctx.project.crate_root_for(from) {
                Some(r) => (r.to_string(), &segs[1..]),
                None => continue,
            },
            "self" => (from.to_string(), &segs[1..]),
            "super" => {
                let mut f = from.to_string();
                let mut i = 0;
                while segs.get(i) == Some(&"super") {
                    // Parent module file: the directory's mod.rs / sibling .rs / crate root.
                    let d = dir_of(&rust_mod_dir(&f)).to_string();
                    let parent = [
                        format!("{d}.rs"),
                        join(&d, "mod.rs"),
                        join(&d, "lib.rs"),
                        join(&d, "main.rs"),
                    ]
                    .into_iter()
                    .find(|p| idx.exists(p));
                    match parent {
                        Some(p) => f = p,
                        None => break,
                    }
                    i += 1;
                }
                (f, &segs[i..])
            }
            krate => match ctx.project.crates.get(krate) {
                Some(r) => (r.clone(), &segs[1..]),
                // Rust 2018: a path may start with a module of the current one (`use order::X`
                // in lib.rs). Otherwise it is std or an external crate.
                None => match rust_module_file(&rust_mod_dir(from), krate, idx) {
                    Some(f) => (f, &segs[1..]),
                    None => continue,
                },
            },
        };
        // Walk down module files as far as they exist; the rest are items inside.
        while let Some((seg, tail)) = rest.split_first() {
            match rust_module_file(&rust_mod_dir(&file), seg, idx) {
                Some(f) => {
                    file = f;
                    rest = tail;
                }
                None => break,
            }
        }
        out.push(file);
    }
    out
}

// ---------------------------------------------------------------- Java / Kotlin

fn jvm(lang: Lang, s: &str, ctx: &Ctx) -> Vec<String> {
    let s = s.trim_start_matches("static ").trim();
    let exts: &[&str] = if lang == Lang::Kotlin {
        &[".kt", ".java"]
    } else {
        &[".java", ".kt"]
    };
    if let Some(pkg) = s.strip_suffix(".*") {
        if let Some(files) = ctx.namespaces.get(pkg) {
            return files.iter().take(50).cloned().collect();
        }
        let dirs = ctx.idx.dirs_with_suffix(&pkg.replace('.', "/"));
        return dirs
            .iter()
            .flat_map(|d| {
                ctx.idx
                    .files_in(d)
                    .iter()
                    .filter(|f| exts.iter().any(|e| f.ends_with(e)))
                    .cloned()
            })
            .take(50)
            .collect();
    }
    // a.b.C, or a.b.C.member for static imports: drop trailing segments until a file matches.
    let mut segs: Vec<&str> = s.split('.').collect();
    while segs.len() >= 2 {
        let rel = segs.join("/");
        for e in exts {
            let c = ctx.idx.with_suffix(&format!("{rel}{e}"));
            if let Some(f) = c.first() {
                return vec![f.to_string()];
            }
        }
        segs.pop();
    }
    Vec::new()
}

// ---------------------------------------------------------------- C / C++

fn c(from: &str, s: &str, ctx: &Ctx) -> Vec<String> {
    let idx = ctx.idx;
    let local = join(dir_of(from), s);
    if idx.exists(&local) {
        return vec![local];
    }
    for d in &ctx.project.include_dirs {
        let p = join(d, s);
        if idx.exists(&p) {
            return vec![p];
        }
    }
    let cands = idx.with_suffix(s);
    if cands.is_empty() {
        return Vec::new();
    }
    // Prefer headers under an include/ directory (linux: <linux/sched.h> → include/linux/sched.h).
    let under_include: Vec<&str> = cands
        .iter()
        .copied()
        .filter(|p| p.starts_with("include/") || p.contains("/include/"))
        .collect();
    let pick = if under_include.is_empty() {
        closest(from, cands)
    } else {
        closest(from, under_include)
    };
    pick.map(String::from).into_iter().collect()
}

// ---------------------------------------------------------------- C#, Ruby, PHP, Swift, Lua

fn namespace_files(ns: &str, ctx: &Ctx, cap: usize) -> Vec<String> {
    ctx.namespaces
        .get(ns)
        .map(|v| v.iter().take(cap).cloned().collect())
        .unwrap_or_default()
}

fn ruby(from: &str, s: &str, ctx: &Ctx) -> Vec<String> {
    let idx = ctx.idx;
    let with_ext = |p: String| {
        if p.ends_with(".rb") {
            p
        } else {
            format!("{p}.rb")
        }
    };
    if s.starts_with("./") || s.starts_with("../") {
        let p = with_ext(join(dir_of(from), s));
        return if idx.exists(&p) { vec![p] } else { Vec::new() };
    }
    let cands = idx.with_suffix(&with_ext(s.to_string()));
    let lib: Vec<&str> = cands
        .iter()
        .copied()
        .filter(|p| p.contains("lib/"))
        .collect();
    let pick = if lib.is_empty() {
        closest(from, cands)
    } else {
        closest(from, lib)
    };
    pick.map(String::from).into_iter().collect()
}

fn php(from: &str, s: &str, ctx: &Ctx) -> Vec<String> {
    let idx = ctx.idx;
    if s.ends_with(".php") {
        let p = join(
            dir_of(from),
            s.trim_start_matches("__DIR__ . ").trim_start_matches('/'),
        );
        if idx.exists(&p) {
            return vec![p];
        }
        return idx
            .with_suffix(s.trim_start_matches("./"))
            .first()
            .map(|p| vec![p.to_string()])
            .unwrap_or_default();
    }
    let s = s.trim_start_matches('\\');
    for (prefix, dir) in &ctx.project.psr4 {
        if let Some(rest) = s.strip_prefix(prefix.as_str()) {
            let p = join(
                dir,
                &format!("{}.php", rest.trim_start_matches('\\').replace('\\', "/")),
            );
            if idx.exists(&p) {
                return vec![p];
            }
        }
    }
    // Without composer mappings: longest path suffix that exists.
    let segs: Vec<&str> = s.split('\\').collect();
    for start in 0..segs.len() {
        let rel = format!("{}.php", segs[start..].join("/"));
        if let Some(p) = closest(from, idx.with_suffix(&rel)) {
            return vec![p.to_string()];
        }
    }
    namespace_files(s, ctx, 50)
}

fn swift(s: &str, ctx: &Ctx) -> Vec<String> {
    let dirs = ctx.idx.dirs_with_suffix(&format!("Sources/{s}"));
    dirs.iter()
        .flat_map(|d| {
            ctx.idx
                .files_in(d)
                .iter()
                .filter(|f| f.ends_with(".swift"))
                .cloned()
        })
        .take(50)
        .collect()
}

fn lua(s: &str, ctx: &Ctx) -> Vec<String> {
    let rel = s.replace('.', "/");
    let mut c = ctx.idx.with_suffix(&format!("{rel}.lua"));
    c.extend(ctx.idx.with_suffix(&format!("{rel}/init.lua")));
    c.first().map(|p| vec![p.to_string()]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(files: &[&str]) -> FileIndex {
        let mut i = FileIndex::default();
        for f in files {
            i.add(f);
        }
        i
    }

    fn imp(path: &str, names: &[&str]) -> Import {
        Import {
            path: path.into(),
            names: names.iter().map(|s| s.to_string()).collect(),
            line: 1,
        }
    }

    fn run(lang: Lang, from: &str, i: &Import, files: &[&str], project: &Project) -> Vec<String> {
        let ix = idx(files);
        let ns = HashMap::new();
        resolve(
            lang,
            from,
            i,
            &Ctx {
                idx: &ix,
                project,
                namespaces: &ns,
            },
        )
    }

    #[test]
    fn suffix_match_requires_equal_text_not_just_equal_length() {
        // Regression: a same-length path used to match any suffix.
        let ix = idx(&[
            "django/utils/version.py",
            "abcdef/ghijk/version.py",
            "x/django/utils/version.py",
        ]);
        let mut hits = ix.with_suffix("django/utils/version.py");
        hits.sort();
        assert_eq!(
            hits,
            vec!["django/utils/version.py", "x/django/utils/version.py"]
        );
    }

    #[test]
    fn typescript_relative_js_extension_and_index() {
        let files = ["src/a.ts", "src/util/index.ts", "src/b.tsx", "src/c.js"];
        let p = Project::default();
        assert_eq!(
            run(
                Lang::TypeScript,
                "src/b.tsx",
                &imp("./a.js", &[]),
                &files,
                &p
            ),
            vec!["src/a.ts"]
        );
        assert_eq!(
            run(
                Lang::TypeScript,
                "src/a.ts",
                &imp("./util", &[]),
                &files,
                &p
            ),
            vec!["src/util/index.ts"]
        );
        assert_eq!(
            run(
                Lang::TypeScript,
                "src/a.ts",
                &imp("./c.js", &[]),
                &files,
                &p
            ),
            vec!["src/c.js"]
        );
        assert!(run(Lang::TypeScript, "src/a.ts", &imp("react", &[]), &files, &p).is_empty());
    }

    #[test]
    fn typescript_paths_and_base_url() {
        let mut p = Project::default();
        p.tsconfigs.insert(
            "".into(),
            crate::project::TsConfig {
                dir: "".into(),
                base_url: Some("src".into()),
                paths: vec![("@x/*".into(), vec!["lib/x/*".into()])],
            },
        );
        let files = ["src/vs/base/common/arrays.ts", "lib/x/y.ts"];
        assert_eq!(
            run(
                Lang::TypeScript,
                "src/vs/a.ts",
                &imp("vs/base/common/arrays", &[]),
                &files,
                &p
            ),
            vec!["src/vs/base/common/arrays.ts"]
        );
        assert_eq!(
            run(
                Lang::TypeScript,
                "src/vs/a.ts",
                &imp("@x/y", &[]),
                &files,
                &p
            ),
            vec!["lib/x/y.ts"]
        );
    }

    #[test]
    fn python_absolute_relative_and_submodules() {
        let files = [
            "django/db/models/__init__.py",
            "django/db/models/query.py",
            "django/db/models/sql/query.py",
            "django/utils/text.py",
            "tests/a.py",
        ];
        let p = Project::default();
        assert_eq!(
            run(
                Lang::Python,
                "tests/a.py",
                &imp("django.db.models", &["QuerySet"]),
                &files,
                &p
            ),
            vec!["django/db/models/__init__.py"]
        );
        assert_eq!(
            run(
                Lang::Python,
                "tests/a.py",
                &imp("django.db.models", &["query"]),
                &files,
                &p
            ),
            vec!["django/db/models/query.py"]
        );
        assert_eq!(
            run(
                Lang::Python,
                "django/db/models/query.py",
                &imp(".sql.query", &["Query"]),
                &files,
                &p
            ),
            vec!["django/db/models/sql/query.py"]
        );
        assert_eq!(
            run(
                Lang::Python,
                "django/db/models/query.py",
                &imp("..", &["models"]),
                &files,
                &p
            ),
            vec!["django/db/models/__init__.py"]
        );
        assert_eq!(
            run(
                Lang::Python,
                "django/db/models/sql/query.py",
                &imp("....utils.text", &["slugify"]),
                &files,
                &p
            ),
            vec!["django/utils/text.py"]
        );
        assert!(run(Lang::Python, "tests/a.py", &imp("os.path", &[]), &files, &p).is_empty());
    }

    #[test]
    fn python_top_level_modules_only_match_at_source_roots() {
        // `import json` must not resolve to a module nested inside a package.
        let files = [
            "django/__init__.py",
            "django/core/__init__.py",
            "django/core/json.py",
            "tests/runner.py",
            "tests/admin/tests.py",
            "tests/admin/__init__.py",
        ];
        let p = Project::default();
        assert!(run(Lang::Python, "django/x.py", &imp("json", &[]), &files, &p).is_empty());
        // tests/ has no __init__.py: it is a source root (django's runtests puts it on sys.path).
        assert_eq!(
            run(
                Lang::Python,
                "tests/runner.py",
                &imp("admin.tests", &[]),
                &files,
                &p
            ),
            vec!["tests/admin/tests.py"]
        );
        assert_eq!(
            run(
                Lang::Python,
                "tests/runner.py",
                &imp("django.core.json", &[]),
                &files,
                &p
            ),
            vec!["django/core/json.py"]
        );
    }

    #[test]
    fn go_modules_and_vendor() {
        let mut p = Project::default();
        p.go_modules.push(("k8s.io/kubernetes".into(), "".into()));
        let files = [
            "pkg/api/types.go",
            "pkg/api/util.go",
            "pkg/api/types_test.go",
            "vendor/github.com/x/y/y.go",
        ];
        assert_eq!(
            run(
                Lang::Go,
                "cmd/main.go",
                &imp("k8s.io/kubernetes/pkg/api", &[]),
                &files,
                &p
            ),
            vec!["pkg/api/types.go", "pkg/api/util.go"]
        );
        assert_eq!(
            run(
                Lang::Go,
                "cmd/main.go",
                &imp("github.com/x/y", &[]),
                &files,
                &p
            ),
            vec!["vendor/github.com/x/y/y.go"]
        );
        assert!(run(Lang::Go, "cmd/main.go", &imp("fmt", &[]), &files, &p).is_empty());
    }

    #[test]
    fn rust_mods_and_use_paths() {
        let mut p = Project::default();
        p.crate_dirs.push(("".into(), "src/lib.rs".into()));
        p.crates.insert("other".into(), "other/src/lib.rs".into());
        let files = [
            "src/lib.rs",
            "src/graph.rs",
            "src/layout/mod.rs",
            "src/layout/squarify.rs",
            "other/src/lib.rs",
        ];
        assert_eq!(
            run(Lang::Rust, "src/lib.rs", &imp("mod:graph", &[]), &files, &p),
            vec!["src/graph.rs"]
        );
        assert_eq!(
            run(
                Lang::Rust,
                "src/layout/mod.rs",
                &imp("mod:squarify", &[]),
                &files,
                &p
            ),
            vec!["src/layout/squarify.rs"]
        );
        assert_eq!(
            run(
                Lang::Rust,
                "src/graph.rs",
                &imp("crate::layout::{squarify::Rect, Layout}", &[]),
                &files,
                &p
            ),
            vec!["src/layout/mod.rs", "src/layout/squarify.rs"]
        );
        assert_eq!(
            run(
                Lang::Rust,
                "src/layout/squarify.rs",
                &imp("super::Layout", &[]),
                &files,
                &p
            ),
            vec!["src/layout/mod.rs"]
        );
        assert_eq!(
            run(
                Lang::Rust,
                "src/graph.rs",
                &imp("other::Thing", &[]),
                &files,
                &p
            ),
            vec!["other/src/lib.rs"]
        );
        assert!(run(
            Lang::Rust,
            "src/graph.rs",
            &imp("std::collections::HashMap", &[]),
            &files,
            &p
        )
        .is_empty());
    }

    #[test]
    fn c_includes_prefer_include_dirs() {
        let files = [
            "include/linux/sched.h",
            "tools/include/linux/sched.h",
            "kernel/sched/core.c",
            "kernel/sched/sched.h",
        ];
        let p = Project::default();
        assert_eq!(
            run(
                Lang::C,
                "kernel/sched/core.c",
                &imp("sched.h", &[]),
                &files,
                &p
            ),
            vec!["kernel/sched/sched.h"]
        );
        assert_eq!(
            run(
                Lang::C,
                "kernel/sched/core.c",
                &imp("linux/sched.h", &[]),
                &files,
                &p
            ),
            vec!["include/linux/sched.h"]
        );
    }

    #[test]
    fn jvm_classes_and_wildcards() {
        let files = [
            "src/main/java/com/acme/core/Order.java",
            "src/main/java/com/acme/core/Line.java",
        ];
        let p = Project::default();
        assert_eq!(
            run(
                Lang::Java,
                "x/A.java",
                &imp("com.acme.core.Order", &[]),
                &files,
                &p
            ),
            vec!["src/main/java/com/acme/core/Order.java"]
        );
        assert_eq!(
            run(
                Lang::Java,
                "x/A.java",
                &imp("static com.acme.core.Order.create", &[]),
                &files,
                &p
            ),
            vec!["src/main/java/com/acme/core/Order.java"]
        );
        assert_eq!(
            run(
                Lang::Java,
                "x/A.java",
                &imp("com.acme.core.*", &[]),
                &files,
                &p
            )
            .len(),
            2
        );
    }
}
