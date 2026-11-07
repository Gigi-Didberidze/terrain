//! History signals from mined commits: churn, ownership by commits, co-change coupling.
//!
//! Coupling between two files is the share of their commits they have in common:
//! degree = shared / mean(commits(a), commits(b)), reported from 5 shared commits up. Merge
//! commits carry no file list, and commits touching more than COUPLING_MAX_FILES files (mass
//! renames, reformatting) are left out of coupling but still count as churn.

use std::collections::HashMap;
use terrain_core::store::CommitRow;

/// Churn window.
pub const CHURN_DAYS: i64 = 90;
/// Minimum shared commits for a coupling to be reported (the plan's support threshold).
pub const MIN_SUPPORT: u32 = 5;
/// Minimum coupling degree kept.
pub const MIN_DEGREE: f32 = 0.3;
/// Commits larger than this do not contribute to coupling.
pub const COUPLING_MAX_FILES: usize = 50;
/// Partners kept per file, strongest first.
const MAX_PARTNERS: usize = 30;

#[derive(Clone, Debug, PartialEq)]
pub struct Partner {
    pub path: String,
    pub degree: f32,
    pub shared: u32,
}

#[derive(Default)]
pub struct History {
    /// HEAD at the time of mining.
    pub head: Option<String>,
    pub commits: usize,
    /// Per file: commits in the last CHURN_DAYS days, and in all mined history.
    pub churn: HashMap<String, u32>,
    pub total: HashMap<String, u32>,
    /// Per file: (top author by commits, that author's share of the file's commits).
    pub owner: HashMap<String, (String, f32)>,
    /// Per file: co-change partners, strongest first.
    pub coupling: HashMap<String, Vec<Partner>>,
    /// Commit id → author, for blame attribution.
    pub authors: HashMap<String, String>,
    /// The 8 authors who own the most files, in colour order (Ownership lens).
    pub top_authors: Vec<String>,
    /// Most recent commits touching one file (Churn lens scale).
    pub max_churn: u32,
    pub millis: f64,
}

impl History {
    pub fn compute(commits: &[CommitRow], live: &dyn Fn(&str) -> bool, now: i64) -> History {
        let t0 = std::time::Instant::now();
        let since = now - CHURN_DAYS * 86_400;
        let mut h = History {
            commits: commits.len(),
            ..Default::default()
        };
        let mut by_author: HashMap<&str, HashMap<&str, u32>> = HashMap::new();
        // File ids for pair counting.
        let mut ids: HashMap<&str, u32> = HashMap::new();
        let mut names: Vec<&str> = Vec::new();
        let mut pairs: HashMap<(u32, u32), u32> = HashMap::new();
        for c in commits {
            h.authors.insert(c.id.clone(), c.author.clone());
            let files: Vec<&str> = c
                .files
                .iter()
                .map(|s| s.as_str())
                .filter(|p| live(p))
                .collect();
            for &f in &files {
                *h.total.entry(f.to_string()).or_default() += 1;
                if c.time >= since {
                    *h.churn.entry(f.to_string()).or_default() += 1;
                }
                *by_author
                    .entry(f)
                    .or_default()
                    .entry(c.author.as_str())
                    .or_default() += 1;
            }
            if files.len() < 2 || files.len() > COUPLING_MAX_FILES {
                continue;
            }
            let mut fids: Vec<u32> = files
                .iter()
                .map(|&f| {
                    *ids.entry(f).or_insert_with(|| {
                        names.push(f);
                        names.len() as u32 - 1
                    })
                })
                .collect();
            fids.sort_unstable();
            for i in 0..fids.len() {
                for j in i + 1..fids.len() {
                    *pairs.entry((fids[i], fids[j])).or_default() += 1;
                }
            }
        }
        // Ownership by commits.
        let mut owned: HashMap<&str, u32> = HashMap::new();
        for (file, authors) in &by_author {
            let total: u32 = authors.values().sum();
            if let Some((a, n)) = authors
                .iter()
                .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
            {
                h.owner
                    .insert(file.to_string(), (a.to_string(), *n as f32 / total as f32));
                *owned.entry(a).or_default() += 1;
            }
        }
        let mut top: Vec<(&str, u32)> = owned.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        h.top_authors = top
            .into_iter()
            .take(8)
            .map(|(a, _)| a.to_string())
            .collect();
        // Coupling.
        for ((a, b), shared) in pairs {
            if shared < MIN_SUPPORT {
                continue;
            }
            let (pa, pb) = (names[a as usize], names[b as usize]);
            let (na, nb) = (h.total[pa] as f32, h.total[pb] as f32);
            let degree = shared as f32 / ((na + nb) / 2.0);
            if degree < MIN_DEGREE {
                continue;
            }
            h.coupling.entry(pa.to_string()).or_default().push(Partner {
                path: pb.to_string(),
                degree,
                shared,
            });
            h.coupling.entry(pb.to_string()).or_default().push(Partner {
                path: pa.to_string(),
                degree,
                shared,
            });
        }
        for v in h.coupling.values_mut() {
            v.sort_by(|a, b| {
                b.degree
                    .total_cmp(&a.degree)
                    .then(b.shared.cmp(&a.shared))
                    .then(a.path.cmp(&b.path))
            });
            v.truncate(MAX_PARTNERS);
        }
        h.max_churn = h.churn.values().copied().max().unwrap_or(0);
        h.millis = t0.elapsed().as_secs_f64() * 1000.0;
        h
    }

    /// Churn lens value: log-scaled against the busiest file, 0 = untouched in the window.
    pub fn churn_lens(&self, path: &str) -> u16 {
        let Some(&c) = self.churn.get(path) else {
            return 0;
        };
        if self.max_churn == 0 {
            return 0;
        }
        let v = (1.0 + c as f64).ln() / (1.0 + self.max_churn as f64).ln();
        ((v * 65535.0).round() as u16).max(1)
    }

    /// Ownership lens value: 1..=8 for the top authors, 0 for anyone else or unknown.
    pub fn owner_lens(&self, path: &str) -> u8 {
        let Some((a, _)) = self.owner.get(path) else {
            return 0;
        };
        self.top_authors
            .iter()
            .position(|t| t == a)
            .map(|i| i as u8 + 1)
            .unwrap_or(0)
    }

    pub fn partners(&self, path: &str) -> &[Partner] {
        self.coupling.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, t: i64, author: &str, files: &[&str]) -> CommitRow {
        CommitRow {
            id: id.into(),
            time: t,
            author: author.into(),
            merge: false,
            files: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn churn_ownership_and_coupling() {
        let day = 86_400;
        let now = 1000 * day;
        let mut commits = Vec::new();
        // a.rs and b.rs change together 6 times; c.rs alone, mostly by bob.
        for i in 0..6 {
            commits.push(c(&format!("p{i}"), now - i * day, "ann", &["a.rs", "b.rs"]));
        }
        commits.push(c("old", now - 200 * day, "bob", &["c.rs"]));
        commits.push(c("c2", now - day, "bob", &["c.rs"]));
        commits.push(c("gone", now - day, "bob", &["deleted.rs", "a.rs"]));
        // A mass change does not create coupling.
        let many: Vec<String> = (0..60).map(|i| format!("f{i}.rs")).collect();
        commits.push(CommitRow {
            id: "mass".into(),
            time: now,
            author: "bot".into(),
            merge: false,
            files: many,
        });
        let h = History::compute(&commits, &|p| p != "deleted.rs", now);
        assert_eq!(h.churn["a.rs"], 7);
        assert_eq!(h.churn["c.rs"], 1, "only the last 90 days");
        assert_eq!(h.total["c.rs"], 2);
        assert_eq!(h.owner["c.rs"].0, "bob");
        let p = h.partners("a.rs");
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].path.as_str(), p[0].shared), ("b.rs", 6));
        assert!((p[0].degree - 6.0 / 6.5).abs() < 1e-6);
        assert!(h.partners("f0.rs").is_empty());
        assert!(!h.churn.contains_key("deleted.rs"));
    }
}
