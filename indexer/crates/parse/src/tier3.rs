//! Tier 3: files without a grammar (YAML, JSON, Dockerfile, Markdown, configs) get edges from
//! path strings that resolve to real files. This returns candidate strings; the engine keeps
//! only those that resolve.

use compass_core::model::Lang;
use regex::Regex;
use std::sync::OnceLock;

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

/// Candidate relative paths mentioned in a file, with the line they appear on.
pub fn path_candidates(lang: Lang, src: &[u8]) -> Vec<(String, u32)> {
    static MD_LINK: OnceLock<Regex> = OnceLock::new();
    static QUOTED: OnceLock<Regex> = OnceLock::new();
    static BARE: OnceLock<Regex> = OnceLock::new();
    let Ok(text) = std::str::from_utf8(src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let ln = i as u32 + 1;
        match lang {
            Lang::Markdown => {
                for c in re(&MD_LINK, r##"\]\(([^)\s#?]+)|(?:src|href)="([^"#?]+)""##)
                    .captures_iter(line)
                {
                    if let Some(m) = c.get(1).or_else(|| c.get(2)) {
                        out.push((m.as_str().to_string(), ln));
                    }
                }
            }
            Lang::Dockerfile => {
                let t = line.trim_start();
                let upper = t.get(..4).map(|s| s.to_ascii_uppercase());
                if matches!(upper.as_deref(), Some("COPY") | Some("ADD ")) {
                    let parts: Vec<&str> = t
                        .split_whitespace()
                        .skip(1)
                        .filter(|p| !p.starts_with("--"))
                        .collect();
                    // Every argument but the last is a source path.
                    for p in parts.iter().take(parts.len().saturating_sub(1)) {
                        out.push((p.trim_matches('"').to_string(), ln));
                    }
                }
            }
            _ => {
                for c in re(&QUOTED, r#"["']([A-Za-z0-9_./@-]{2,200})["']"#).captures_iter(line) {
                    out.push((c[1].to_string(), ln));
                }
                if matches!(lang, Lang::Yaml | Lang::Toml | Lang::Shell) {
                    for c in re(&BARE, r"(?:^|[\s:=])(\.{0,2}/?[A-Za-z0-9_-]+(?:/[A-Za-z0-9_.-]+)+\.[A-Za-z0-9]{1,6})\b").captures_iter(line) {
                        out.push((c[1].to_string(), ln));
                    }
                }
            }
        }
    }
    out.retain(|(p, _)| looks_like_path(p));
    out.sort();
    out.dedup();
    out
}

fn looks_like_path(p: &str) -> bool {
    !p.contains("://")
        && !p.starts_with("mailto:")
        && (p.contains('/') || p.contains('.'))
        && !p.chars().all(|c| c.is_ascii_digit() || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_links_and_images() {
        let c = path_candidates(
            Lang::Markdown,
            b"See [guide](docs/guide.md#intro) and <img src=\"img/a.png\">.\n[web](https://x.y)",
        );
        let paths: Vec<_> = c.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["docs/guide.md", "img/a.png"]);
    }

    #[test]
    fn dockerfile_copy_sources() {
        let c = path_candidates(
            Lang::Dockerfile,
            b"FROM x\nCOPY --chown=a requirements.txt setup.py /app/\nADD src/ /app/src\n",
        );
        let paths: Vec<_> = c.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["requirements.txt", "setup.py", "src/"]);
    }

    #[test]
    fn config_strings() {
        let c = path_candidates(
            Lang::Yaml,
            b"run: python scripts/build.py\nfile: './config/app.json'\nversion: '3.8'\n",
        );
        let paths: Vec<_> = c.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"scripts/build.py"));
        assert!(paths.contains(&"./config/app.json"));
        assert!(!paths.contains(&"3.8"));
    }
}
