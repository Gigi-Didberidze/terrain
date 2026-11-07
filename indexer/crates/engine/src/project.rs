//! Project configuration that import resolution depends on, read from the workspace:
//! tsconfig/jsconfig (baseUrl, paths), package.json names, go.mod modules, Cargo crates,
//! composer.json PSR-4 roots and compile_commands.json include directories.

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct TsConfig {
    /// Directory of the tsconfig (workspace-relative, "" for the root).
    pub dir: String,
    /// baseUrl, workspace-relative.
    pub base_url: Option<String>,
    /// paths patterns → targets (relative to base_url, or to `dir` without one).
    pub paths: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, Default)]
pub struct Project {
    /// Keyed by directory; the nearest ancestor applies.
    pub tsconfigs: HashMap<String, TsConfig>,
    /// package.json "name" → (dir, entry file relative to dir if declared).
    pub packages: HashMap<String, (String, Option<String>)>,
    /// Go module path → directory.
    pub go_modules: Vec<(String, String)>,
    /// Rust crate name (underscored) → crate root file (src/lib.rs or src/main.rs).
    pub crates: HashMap<String, String>,
    /// Directory → crate root file, for `crate::` paths.
    pub crate_dirs: Vec<(String, String)>,
    /// PHP namespace prefix → directory.
    pub psr4: Vec<(String, String)>,
    /// C/C++ include directories, workspace-relative.
    pub include_dirs: Vec<String>,
}

fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

pub fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Strips // and /* */ comments and trailing commas so tsconfig-style JSONC parses.
pub fn strip_jsonc(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    strip_trailing_commas(&String::from_utf8_lossy(&out))
}

fn strip_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    for (i, &c) in chars.iter().enumerate() {
        if c == '"' && (i == 0 || chars[i - 1] != '\\') {
            in_str = !in_str;
        }
        if !in_str && c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn load_tsconfig(root: &Path, rel: &str, depth: u32) -> Option<TsConfig> {
    let text = read(root, rel)?;
    let v: serde_json::Value = serde_json::from_str(&strip_jsonc(&text)).ok()?;
    let dir = dir_of(rel).to_string();
    // Follow "extends" (relative only) for inherited baseUrl/paths.
    let mut cfg = TsConfig {
        dir: dir.clone(),
        ..Default::default()
    };
    if let (Some(ext), true) = (v.get("extends").and_then(|e| e.as_str()), depth < 4) {
        if ext.starts_with('.') {
            let mut p = join(&dir, ext);
            if !p.ends_with(".json") {
                p.push_str(".json");
            }
            if let Some(parent) = load_tsconfig(root, &p, depth + 1) {
                cfg.base_url = parent.base_url;
                cfg.paths = parent.paths;
            }
        }
    }
    if let Some(co) = v.get("compilerOptions") {
        if let Some(b) = co.get("baseUrl").and_then(|b| b.as_str()) {
            cfg.base_url = Some(join(&dir, b));
        }
        if let Some(paths) = co.get("paths").and_then(|p| p.as_object()) {
            let base = cfg.base_url.clone().unwrap_or_else(|| dir.clone());
            cfg.paths = paths
                .iter()
                .map(|(k, v)| {
                    let targets = v
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| t.as_str())
                                .map(|t| join(&base, t))
                                .collect()
                        })
                        .unwrap_or_default();
                    (k.clone(), targets)
                })
                .collect();
        }
    }
    Some(cfg)
}

impl Project {
    /// Reads every config file among `paths` (workspace-relative file list).
    pub fn discover<'a>(root: &Path, paths: impl Iterator<Item = &'a str>) -> Project {
        let mut p = Project::default();
        let mut cargo_tomls = Vec::new();
        for path in paths {
            let name = path.rsplit('/').next().unwrap_or(path);
            match name {
                "tsconfig.json" | "jsconfig.json" => {
                    if let Some(c) = load_tsconfig(root, path, 0) {
                        p.tsconfigs.entry(c.dir.clone()).or_insert(c);
                    }
                }
                "package.json" => {
                    if let Some(v) = read(root, path)
                        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    {
                        if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                            let entry = ["types", "module", "main"]
                                .iter()
                                .find_map(|k| v.get(*k).and_then(|m| m.as_str()))
                                .map(|s| s.trim_start_matches("./").to_string());
                            p.packages
                                .insert(n.to_string(), (dir_of(path).to_string(), entry));
                        }
                    }
                }
                "go.mod" => {
                    if let Some(t) = read(root, path) {
                        if let Some(m) = t.lines().find_map(|l| l.trim().strip_prefix("module ")) {
                            p.go_modules.push((
                                m.trim().trim_matches('"').to_string(),
                                dir_of(path).to_string(),
                            ));
                        }
                    }
                }
                "Cargo.toml" => cargo_tomls.push(path.to_string()),
                "composer.json" => {
                    if let Some(v) = read(root, path)
                        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    {
                        for key in ["autoload", "autoload-dev"] {
                            if let Some(m) = v
                                .get(key)
                                .and_then(|a| a.get("psr-4"))
                                .and_then(|m| m.as_object())
                            {
                                for (ns, d) in m {
                                    let dirs: Vec<String> = match d {
                                        serde_json::Value::String(s) => vec![s.clone()],
                                        serde_json::Value::Array(a) => a
                                            .iter()
                                            .filter_map(|x| x.as_str().map(String::from))
                                            .collect(),
                                        _ => vec![],
                                    };
                                    for dd in dirs {
                                        p.psr4.push((
                                            ns.trim_end_matches('\\').to_string(),
                                            join(dir_of(path), &dd),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                "compile_commands.json" => {
                    if let Some(v) = read(root, path)
                        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    {
                        let mut seen = std::collections::HashSet::new();
                        for e in v.as_array().into_iter().flatten().take(20_000) {
                            let args: Vec<String> =
                                if let Some(a) = e.get("arguments").and_then(|a| a.as_array()) {
                                    a.iter()
                                        .filter_map(|x| x.as_str().map(String::from))
                                        .collect()
                                } else {
                                    e.get("command")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("")
                                        .split_whitespace()
                                        .map(String::from)
                                        .collect()
                                };
                            let dir = e.get("directory").and_then(|d| d.as_str()).unwrap_or("");
                            let mut it = args.iter();
                            while let Some(a) = it.next() {
                                let inc = if a == "-I" {
                                    it.next().cloned()
                                } else {
                                    a.strip_prefix("-I").map(String::from)
                                };
                                if let Some(inc) = inc {
                                    let abs = if Path::new(&inc).is_absolute() {
                                        inc
                                    } else {
                                        format!("{dir}/{inc}")
                                    };
                                    if let Ok(rel) = Path::new(&abs).strip_prefix(root) {
                                        let r = rel.to_string_lossy().replace('\\', "/");
                                        if seen.insert(r.clone()) {
                                            p.include_dirs.push(r);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for path in cargo_tomls {
            let Some(t) = read(root, &path) else { continue };
            let dir = dir_of(&path).to_string();
            let name = t.split("[package]").nth(1).and_then(|sec| {
                sec.lines().find_map(|l| {
                    l.trim().strip_prefix("name").map(|r| {
                        r.trim_start()
                            .trim_start_matches('=')
                            .trim()
                            .trim_matches('"')
                            .to_string()
                    })
                })
            });
            let prefix = if dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            };
            let root_file = [
                format!("{prefix}src/lib.rs"),
                format!("{prefix}src/main.rs"),
            ]
            .into_iter()
            .find(|f| root.join(f).exists());
            if let (Some(n), Some(rf)) = (name, root_file) {
                p.crates.insert(n.replace('-', "_"), rf.clone());
                p.crate_dirs.push((dir, rf));
            }
        }
        // Longest prefix first, so nested modules/crates win.
        p.go_modules.sort_by_key(|m| std::cmp::Reverse(m.0.len()));
        p.crate_dirs.sort_by_key(|c| std::cmp::Reverse(c.0.len()));
        p.psr4.sort_by_key(|n| std::cmp::Reverse(n.0.len()));
        p
    }

    /// Nearest tsconfig/jsconfig above a file.
    pub fn tsconfig_for(&self, file: &str) -> Option<&TsConfig> {
        let mut dir = dir_of(file);
        loop {
            if let Some(c) = self.tsconfigs.get(dir) {
                return Some(c);
            }
            if dir.is_empty() {
                return None;
            }
            dir = dir_of(dir);
        }
    }

    /// Crate root file for a Rust source file.
    pub fn crate_root_for(&self, file: &str) -> Option<&str> {
        self.crate_dirs
            .iter()
            .find(|(d, _)| d.is_empty() || file.starts_with(&format!("{d}/")))
            .map(|(_, r)| r.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_paths() {
        assert_eq!(join("src/vs/base", "../common/x"), "src/vs/common/x");
        assert_eq!(join("", "./a/b"), "a/b");
    }

    #[test]
    fn parses_jsonc() {
        let v: serde_json::Value = serde_json::from_str(&strip_jsonc(
            "{\n // c\n \"a\": \"//x\", /* b */ \"b\": [1,2,],\n}",
        ))
        .unwrap();
        assert_eq!(v["a"], "//x");
        assert_eq!(v["b"][1], 2);
    }

    #[test]
    fn reads_configs() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("web/src")).unwrap();
        std::fs::create_dir_all(r.join("svc/src")).unwrap();
        std::fs::write(
            r.join("web/tsconfig.json"),
            r#"{ "compilerOptions": { "baseUrl": "src", "paths": { "@app/*": ["app/*"] } } }"#,
        )
        .unwrap();
        std::fs::write(r.join("go.mod"), "module example.com/svc\n\ngo 1.22\n").unwrap();
        std::fs::write(r.join("svc/Cargo.toml"), "[package]\nname = \"my-svc\"\n").unwrap();
        std::fs::write(r.join("svc/src/lib.rs"), "").unwrap();
        let files = ["web/tsconfig.json", "go.mod", "svc/Cargo.toml"];
        let p = Project::discover(r, files.iter().copied());
        let ts = p.tsconfig_for("web/src/app/x.ts").unwrap();
        assert_eq!(ts.base_url.as_deref(), Some("web/src"));
        assert_eq!(
            ts.paths[0],
            ("@app/*".to_string(), vec!["web/src/app/*".to_string()])
        );
        assert_eq!(
            p.go_modules,
            vec![("example.com/svc".to_string(), "".to_string())]
        );
        assert_eq!(
            p.crates.get("my_svc").map(String::as_str),
            Some("svc/src/lib.rs")
        );
        assert_eq!(p.crate_root_for("svc/src/a/b.rs"), Some("svc/src/lib.rs"));
    }
}
