//! Review mode: what the current branch changed (committed since its base, plus uncommitted and
//! untracked work) and what depends on it. Changed files are ranked by load-bearing score: the
//! files a reviewer should read first.

use crate::analysis::Analysis;
use crate::context::BLAST_DEPTH;
use std::collections::HashSet;
use terrain_core::Graph;

#[derive(Clone, Debug, Default)]
pub struct Review {
    /// Short name of the base branch ("origin/main"); None outside git or without a base.
    pub base_name: Option<String>,
    /// Merge base commit.
    pub base: Option<String>,
    /// Changed paths as git reports them (including deleted files and files not on the map).
    pub paths: Vec<String>,
    /// Paths changed in commits since the merge base (the rest are uncommitted).
    pub committed: HashSet<String>,
    /// Changed files on the map, by load-bearing score (highest first).
    pub changed: Vec<u32>,
    /// Files outside the change that depend on it within BLAST_DEPTH steps, by load-bearing score.
    pub blast: Vec<u32>,
    changed_set: HashSet<u32>,
    blast_set: HashSet<u32>,
    pub millis: f64,
}

impl Review {
    /// Resolves paths to files on the map and computes the blast radius (multi-source BFS over
    /// dependents, so a large change costs one walk, not one per file).
    pub fn build(
        g: &Graph,
        a: &Analysis,
        base_name: Option<String>,
        base: Option<String>,
        committed: Vec<String>,
        uncommitted: Vec<String>,
    ) -> Review {
        let t0 = std::time::Instant::now();
        let committed_set: HashSet<String> = committed.iter().cloned().collect();
        let mut paths: Vec<String> = committed.into_iter().chain(uncommitted).collect();
        paths.sort();
        paths.dedup();
        let by_rank = |v: &mut Vec<u32>| {
            v.sort_by(|x, y| a.rank_of(*y).total_cmp(&a.rank_of(*x)).then(x.cmp(y)));
        };
        let mut changed: Vec<u32> = paths.iter().filter_map(|p| g.slot_of_path(p)).collect();
        changed.sort_unstable();
        changed.dedup();
        let changed_set: HashSet<u32> = changed.iter().copied().collect();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut frontier: Vec<u32> = changed.iter().filter_map(|&f| a.file_index(f)).collect();
        seen.extend(frontier.iter().copied());
        let mut blast = Vec::new();
        for _ in 0..BLAST_DEPTH {
            let mut next = Vec::new();
            for &i in &frontier {
                for (t, _) in a.dependents.neighbors(i) {
                    if seen.insert(t) {
                        next.push(t);
                        blast.push(a.files[t as usize]);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        by_rank(&mut changed);
        by_rank(&mut blast);
        let blast_set = blast.iter().copied().collect();
        Review {
            base_name,
            base,
            paths,
            committed: committed_set,
            changed,
            blast,
            changed_set,
            blast_set,
            millis: t0.elapsed().as_secs_f64() * 1000.0,
        }
    }

    /// Same change, recomputed against a new graph and analysis.
    pub fn rebuild(&self, g: &Graph, a: &Analysis) -> Review {
        let (committed, uncommitted): (Vec<String>, Vec<String>) = self
            .paths
            .iter()
            .cloned()
            .partition(|p| self.committed.contains(p));
        Review::build(
            g,
            a,
            self.base_name.clone(),
            self.base.clone(),
            committed,
            uncommitted,
        )
    }

    pub fn is_changed(&self, file: u32) -> bool {
        self.changed_set.contains(&file)
    }

    pub fn in_blast(&self, file: u32) -> bool {
        self.blast_set.contains(&file)
    }
}
