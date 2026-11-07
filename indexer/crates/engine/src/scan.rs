//! Discover and Hash stages: walk the workspace honouring .gitignore, then read each file once to
//! get its BLAKE3 hash, lines of code and generated-file status.

use compass_core::model::{flags, Lang};
use ignore::{WalkBuilder, WalkState};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const DEFAULT_SIZE_CAP: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Discovered {
    /// Workspace-relative path with '/' separators.
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

/// Directory names that are never part of the map.
fn is_excluded_dir(name: &str) -> bool {
    matches!(name, ".git" | "node_modules" | ".hg" | ".svn")
}

pub fn rel_path(root: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    let mut s = String::with_capacity(rel.as_os_str().len());
    for (i, c) in rel.components().enumerate() {
        if i > 0 {
            s.push('/');
        }
        s.push_str(&c.as_os_str().to_string_lossy());
    }
    (!s.is_empty()).then_some(s)
}

pub fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Walks the workspace in parallel. Calls `on_batch` with chunks as they are found so the map can
/// show directories and files within the first seconds.
pub fn discover(root: &Path, mut on_batch: impl FnMut(Vec<Discovered>)) -> usize {
    let (tx, rx) = std::sync::mpsc::channel::<Discovered>();
    let root_buf: PathBuf = root.to_path_buf();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && is_excluded_dir(&e.file_name().to_string_lossy()))
        })
        .threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(12),
        )
        .build_parallel();
    let handle = std::thread::spawn(move || {
        walker.run(|| {
            let tx = tx.clone();
            let root = root_buf.clone();
            Box::new(move |entry| {
                let Ok(e) = entry else {
                    return WalkState::Continue;
                };
                if !e.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                let Some(path) = rel_path(&root, e.path()) else {
                    return WalkState::Continue;
                };
                let Ok(meta) = e.metadata() else {
                    return WalkState::Continue;
                };
                let _ = tx.send(Discovered {
                    path,
                    size: meta.len(),
                    mtime: mtime_ns(&meta),
                });
                WalkState::Continue
            })
        });
    });
    let mut total = 0;
    let mut batch = Vec::with_capacity(4096);
    let mut last_flush = std::time::Instant::now();
    for d in rx {
        batch.push(d);
        total += 1;
        if batch.len() >= 4096 || last_flush.elapsed().as_millis() > 100 {
            on_batch(std::mem::take(&mut batch));
            last_flush = std::time::Instant::now();
        }
    }
    if !batch.is_empty() {
        on_batch(batch);
    }
    let _ = handle.join();
    total
}

#[derive(Clone, Debug)]
pub struct Scanned {
    pub hash: [u8; 32],
    pub loc: u32,
    pub flags: u16,
    pub lang: Lang,
    /// File content, kept only when it will be parsed next.
    pub content: Option<Vec<u8>>,
}

/// Rough lines-of-code estimate for files we don't read (over the size cap).
pub fn estimate_loc(size: u64) -> u32 {
    (size / 40).min(u32::MAX as u64) as u32
}

/// Reads a file, hashes it and classifies it. Returns None if it can't be read.
pub fn scan_file(
    root: &Path,
    d: &Discovered,
    size_cap: u64,
    keep_content: bool,
) -> Option<Scanned> {
    let lang = Lang::from_path(&d.path);
    if d.size > size_cap {
        let mut h = blake3::Hasher::new();
        h.update(d.path.as_bytes());
        h.update(&d.size.to_le_bytes());
        h.update(&d.mtime.to_le_bytes());
        let loc = estimate_loc(d.size);
        // Large files are the likeliest data files of all: classify them here too.
        let data = if counts_as_data(&d.path, loc, lang, true) {
            flags::DATA
        } else {
            0
        };
        return Some(Scanned {
            hash: *h.finalize().as_bytes(),
            loc,
            flags: flags::TOO_LARGE | data,
            lang,
            content: None,
        });
    }
    let bytes = std::fs::read(root.join(&d.path)).ok()?;
    let hash = *blake3::hash(&bytes).as_bytes();
    let binary = memchr::memchr(0, &bytes[..bytes.len().min(8000)]).is_some();
    if binary {
        return Some(Scanned {
            hash,
            loc: 0,
            flags: flags::GENERATED,
            lang: Lang::Unknown,
            content: None,
        });
    }
    let loc = count_loc(&bytes);
    let mut fl = 0;
    if is_generated(&d.path, &bytes) {
        fl |= flags::GENERATED;
    }
    if is_test_path(&d.path) {
        fl |= flags::TEST;
    }
    if is_data_file(&d.path, loc) {
        fl |= flags::DATA;
    }
    // Data files are not scanned for path mentions: slow on big files and meaningless.
    let content = (keep_content && fl & (flags::GENERATED | flags::DATA) == 0 && lang.tier() <= 3)
        .then_some(bytes);
    Some(Scanned {
        hash,
        loc,
        flags: fl,
        lang,
        content,
    })
}

/// Non-blank lines.
pub fn count_loc(bytes: &[u8]) -> u32 {
    let mut loc = 0u32;
    let mut blank = true;
    for &b in bytes {
        match b {
            b'\n' => {
                if !blank {
                    loc += 1;
                }
                blank = true;
            }
            b' ' | b'\t' | b'\r' | 0x0c => {}
            _ => blank = false,
        }
    }
    if !blank {
        loc += 1;
    }
    loc
}

/// Generated-file detection: vendored paths, header markers, minified content.
pub fn is_generated(path: &str, bytes: &[u8]) -> bool {
    if path
        .split('/')
        .any(|seg| seg == "vendor" || seg == "third_party_vendored")
    {
        return true;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.contains(".min.")
        || name.ends_with(".pb.go")
        || name.ends_with("_pb2.py")
        || name.ends_with(".g.dart")
    {
        return true;
    }
    if matches!(
        name,
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "Cargo.lock"
            | "go.sum"
            | "poetry.lock"
            | "Gemfile.lock"
            | "composer.lock"
    ) {
        return true;
    }
    let head = &bytes[..bytes.len().min(1024)];
    let head_s = String::from_utf8_lossy(head);
    let markers = [
        "@generated",
        "Code generated",
        "DO NOT EDIT",
        "auto-generated",
        "autogenerated",
        "This file is generated",
    ];
    if markers.iter().any(|m| head_s.contains(m)) {
        return true;
    }
    // Minified: very long lines in the first 4 KiB.
    let sample = &bytes[..bytes.len().min(4096)];
    if sample.len() >= 4096 && !sample.contains(&b'\n') {
        return true;
    }
    false
}

/// Lines above which text formats count as data rather than hand-written config.
const DATA_LINES: u32 = 2000;

/// Data rather than code: tables, logs, datasets, images-as-text. Area on the map is meant to be
/// code, so these are capped (DECISIONS.md D34).
/// The data classification used everywhere: known data formats, plus any file over the size
/// cap whose language Compass does not recognise (it was never read, so its "lines" are only
/// an estimate from its byte size).
pub fn counts_as_data(path: &str, loc: u32, lang: Lang, too_large: bool) -> bool {
    is_data_file(path, loc) || (too_large && lang == Lang::Unknown)
}

pub fn is_data_file(path: &str, loc: u32) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let always = [
        "csv",
        "tsv",
        "psv",
        "jsonl",
        "ndjson",
        "parquet",
        "avro",
        "orc",
        "feather",
        "arrow",
        "log",
        "dat",
        "geojson",
        "topojson",
        "svg",
        "snap",
        "pbf",
        "mtx",
        "arff",
        "xls",
        "xlsx",
        "ods",
        "sav",
        "dta",
        "rds",
        "pkl",
        "npy",
        "npz",
        "h5",
        "hdf5",
        "db",
        "sqlite",
        "sqlite3",
        "tfrecord",
        "vcf",
        "fasta",
        "fastq",
        "sam",
        "gpx",
        "kml",
        "srt",
        "vtt",
        "po",
        "pot",
        "mo",
        // Documents, images, media, archives, binaries and model weights: never code.
        "pdf",
        "doc",
        "docx",
        "ppt",
        "pptx",
        "key",
        "pages",
        "numbers",
        "rtf",
        "epub",
        "png",
        "jpg",
        "jpeg",
        "gif",
        "webp",
        "bmp",
        "ico",
        "icns",
        "tif",
        "tiff",
        "psd",
        "ai",
        "eps",
        "heic",
        "mp3",
        "mp4",
        "m4a",
        "mov",
        "avi",
        "mkv",
        "wav",
        "flac",
        "ogg",
        "webm",
        "zip",
        "gz",
        "tgz",
        "bz2",
        "xz",
        "7z",
        "rar",
        "tar",
        "zst",
        "jar",
        "war",
        "whl",
        "egg",
        "so",
        "dll",
        "dylib",
        "exe",
        "bin",
        "o",
        "a",
        "lib",
        "class",
        "pyc",
        "wasm",
        "woff",
        "woff2",
        "ttf",
        "otf",
        "eot",
        "onnx",
        "pt",
        "pth",
        "ckpt",
        "safetensors",
        "pb",
        "tflite",
        "joblib",
        "model",
        "gguf",
        "eml",
        "msg",
        "mbox",
        "ics",
        "vcard",
    ];
    if always.contains(&ext) {
        return true;
    }
    // Lock files: generated manifests, not code anyone writes.
    if matches!(
        name.as_str(),
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "poetry.lock"
            | "uv.lock"
            | "pipfile.lock"
            | "cargo.lock"
            | "go.sum"
            | "gemfile.lock"
            | "composer.lock"
            | "pubspec.lock"
            | "mix.lock"
            | "flake.lock"
            | "bun.lockb"
    ) {
        return true;
    }
    // Text formats that are config when small and data when large.
    matches!(
        ext,
        // Notebooks too: saved outputs inflate their line count far beyond the code.
        "json" | "xml" | "txt" | "yaml" | "yml" | "sql" | "toml" | "ini" | "properties" | "ipynb"
    ) && loc > DATA_LINES
}

pub fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    path.split('/').any(|s| {
        matches!(
            s,
            "test" | "tests" | "__tests__" | "spec" | "testdata" | "testing"
        )
    }) || stem.starts_with("test_")
        || stem.ends_with("_test")
        || stem.ends_with("_spec")
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
        || name.contains(".test.")
        || name.contains(".spec.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_non_blank_lines() {
        assert_eq!(count_loc(b""), 0);
        assert_eq!(count_loc(b"a\n\n  \nb"), 2);
        assert_eq!(count_loc(b"a\r\n\r\nb\r\n"), 2);
    }

    #[test]
    fn detects_generated_files() {
        assert!(is_generated("x/vendor/y.go", b"package y"));
        assert!(is_generated(
            "a.go",
            b"// Code generated by protoc. DO NOT EDIT.\npackage a"
        ));
        assert!(is_generated("app.min.js", b"x"));
        assert!(!is_generated("src/app.js", b"export const a = 1;\n"));
        assert!(is_generated("big.js", &[b'x'; 5000]));
    }

    #[test]
    fn files_over_the_size_cap_are_classified_too() {
        // Regression: a 16 MB CSV took the size-cap shortcut and skipped classification.
        let dir = tempfile::tempdir().unwrap();
        let d = Discovered {
            path: "data/geo_features.csv".into(),
            size: 16 * 1024 * 1024,
            mtime: 1,
        };
        let s = scan_file(dir.path(), &d, DEFAULT_SIZE_CAP, false).unwrap();
        assert!(s.flags & flags::DATA != 0 && s.flags & flags::TOO_LARGE != 0);
        // A large PDF is never read; it must not be sized by its byte count.
        let pdf = Discovered {
            path: "reports/analysis.pdf".into(),
            size: 2 * 1024 * 1024,
            mtime: 1,
        };
        assert!(
            scan_file(dir.path(), &pdf, DEFAULT_SIZE_CAP, false)
                .unwrap()
                .flags
                & flags::DATA
                != 0
        );
        assert!(counts_as_data(
            "blobs/unknown.xyz",
            50_000,
            Lang::Unknown,
            true
        ));
        assert!(!counts_as_data("src/huge.py", 50_000, Lang::Python, true));
        assert!(is_data_file("poetry.lock", 10));
    }

    #[test]
    fn detects_data_files() {
        assert!(is_data_file("data/prices.csv", 3));
        assert!(is_data_file("logs/server.LOG", 10));
        assert!(is_data_file("fixtures/dump.json", 50_000));
        assert!(!is_data_file("package.json", 40));
        assert!(!is_data_file("src/main.py", 50_000));
        assert!(!is_data_file("Makefile", 10));
    }

    #[test]
    fn detects_test_paths() {
        assert!(is_test_path("tests/test_models.py"));
        assert!(is_test_path("pkg/foo_test.go"));
        assert!(is_test_path("src/a.test.ts"));
        assert!(is_test_path("src/FooTest.java"));
        assert!(!is_test_path("src/contest.rs"));
    }

    #[test]
    fn discover_honours_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        std::fs::create_dir_all(root.join("ignored")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("ignored/a.rs"), "x").unwrap();
        std::fs::write(root.join("node_modules/x/i.js"), "x").unwrap();
        std::fs::write(root.join("src/b.rs"), "x").unwrap();
        let mut found = Vec::new();
        discover(root, |b| found.extend(b.into_iter().map(|d| d.path)));
        found.sort();
        assert_eq!(
            found,
            vec![".gitignore".to_string(), "src/b.rs".to_string()]
        );
    }
}
