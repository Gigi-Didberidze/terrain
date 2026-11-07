//! compass-git: history mining with gix (no shelling out to `git`).
//!
//! Reads each commit's changed files once, in parallel, and incrementally from the last mined
//! commit afterwards. Blame is computed on demand for a single file.

use gix::bstr::ByteSlice;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Commit {
    /// Hex object id.
    pub id: String,
    /// Commit time, seconds since the epoch.
    pub time: i64,
    /// Author email (stable across name spellings), falling back to the name.
    pub author: String,
    /// Workspace-relative paths changed against the first parent (empty for merges).
    pub files: Vec<String>,
    /// Merge commits are kept for completeness but carry no file list.
    pub merge: bool,
}

/// A repository opened for a workspace that may be a subdirectory of it.
pub struct Repo {
    repo: gix::ThreadSafeRepository,
    /// Workspace path relative to the repository work tree ("" when they coincide).
    prefix: String,
}

#[derive(Debug, Default)]
pub struct Mined {
    pub head: Option<String>,
    /// Newest first.
    pub commits: Vec<Commit>,
    /// Previously mined commits no longer reachable from HEAD (after a branch switch, reset or
    /// rebase): ids to forget.
    pub dropped: Vec<String>,
    /// The previous head is unknown or unrelated: `commits` is the whole history, forget the rest.
    pub replace: bool,
}

fn rel_prefix(workdir: &Path, workspace: &Path) -> String {
    let w = dunce(workspace);
    let r = dunce(workdir);
    w.strip_prefix(&r)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// Canonicalises without Windows verbatim prefixes so prefix stripping works.
fn dunce(p: &Path) -> PathBuf {
    let c = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let s = c.to_string_lossy();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

impl Repo {
    /// Opens the repository containing `workspace`, if there is one.
    pub fn open(workspace: &Path) -> Option<Repo> {
        let repo = gix::discover(workspace).ok()?;
        let workdir = repo.workdir()?.to_path_buf();
        let prefix = rel_prefix(&workdir, workspace);
        Some(Repo {
            repo: repo.into_sync(),
            prefix,
        })
    }

    pub fn head(&self) -> Option<String> {
        let repo = self.repo.to_thread_local();
        repo.head_id().ok().map(|id| id.to_string())
    }

    /// Repository path → workspace path, or None when outside the workspace.
    fn to_workspace(&self, path: &str) -> Option<String> {
        if self.prefix.is_empty() {
            return Some(path.to_string());
        }
        path.strip_prefix(&self.prefix)
            .and_then(|p| p.strip_prefix('/'))
            .map(String::from)
    }

    /// Mines commits reachable from HEAD that are not reachable from `since` (all when None).
    pub fn mine(
        &self,
        since: Option<&str>,
        progress: &(dyn Fn(usize, usize) + Sync),
    ) -> Result<Mined> {
        let repo = self.repo.to_thread_local();
        let Ok(head) = repo.head_id() else {
            return Ok(Mined::default());
        };
        let head_hex = head.to_string();
        if since == Some(head_hex.as_str()) {
            return Ok(Mined {
                head: Some(head_hex),
                ..Default::default()
            });
        }
        // Commits since the merge base of the old and new head; the old side past the base is
        // what a branch switch or rewrite dropped.
        let mut hidden = None;
        let mut dropped = Vec::new();
        if let Some(old) = since.and_then(|s| gix::ObjectId::from_hex(s.as_bytes()).ok()) {
            if repo.find_object(old).is_ok() {
                if let Ok(Some(base)) = repo.merge_base(old, head.detach()) {
                    let base = base.detach();
                    if base != old {
                        for info in repo.rev_walk([old]).with_hidden([base]).all()? {
                            dropped.push(info?.id.to_string());
                        }
                    }
                    hidden = Some(base);
                }
            }
        }
        let replace = since.is_some() && hidden.is_none();
        let mut walk = repo.rev_walk([head.detach()]);
        if let Some(base) = hidden {
            walk = walk.with_hidden([base]);
        }
        // Sequential walk: ids, parents, time, author. Cheap compared with tree diffs.
        struct Meta {
            id: gix::ObjectId,
            parent: Option<gix::ObjectId>,
            merge: bool,
            time: i64,
            author: String,
        }
        let mut metas = Vec::new();
        for info in walk.all()? {
            let info = info?;
            let commit = info.object()?;
            let parents: Vec<gix::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
            let sig = commit.author()?;
            let author = if sig.email.is_empty() {
                sig.name.to_str_lossy().into_owned()
            } else {
                sig.email.to_str_lossy().to_lowercase()
            };
            let time = commit.time().map(|t| t.seconds).unwrap_or(0);
            metas.push(Meta {
                id: info.id,
                parent: parents.first().copied(),
                merge: parents.len() > 1,
                time,
                author,
            });
        }
        let total = metas.len();
        let done = std::sync::atomic::AtomicUsize::new(0);
        // Parallel tree diffs: one thread-local repository handle per worker.
        let commits: Vec<Commit> = metas
            .par_chunks(256)
            .map_init(
                || self.repo.to_thread_local(),
                |repo, chunk| {
                    let mut out = Vec::with_capacity(chunk.len());
                    for m in chunk {
                        let files = if m.merge {
                            Vec::new()
                        } else {
                            self.changed_files(repo, m.id, m.parent).unwrap_or_default()
                        };
                        out.push(Commit {
                            id: m.id.to_string(),
                            time: m.time,
                            author: m.author.clone(),
                            files,
                            merge: m.merge,
                        });
                    }
                    let n = done.fetch_add(chunk.len(), std::sync::atomic::Ordering::Relaxed)
                        + chunk.len();
                    progress(n, total);
                    out
                },
            )
            .flatten()
            .collect();
        Ok(Mined {
            head: Some(head_hex),
            commits,
            dropped,
            replace,
        })
    }

    fn changed_files(
        &self,
        repo: &gix::Repository,
        id: gix::ObjectId,
        parent: Option<gix::ObjectId>,
    ) -> Result<Vec<String>> {
        let tree = repo.find_commit(id)?.tree()?;
        let parent_tree = match parent {
            Some(p) => Some(repo.find_commit(p)?.tree()?),
            None => None,
        };
        // No rename tracking: it needs blob comparisons and the paths are what we want.
        let mut opts = gix::diff::Options::default().with_rewrites(None);
        opts.track_path();
        let changes = repo.diff_tree_to_tree(parent_tree.as_ref(), &tree, Some(opts))?;
        let mut files: Vec<String> = changes
            .iter()
            .filter(|c| !matches!(c.entry_mode().kind(), gix::object::tree::EntryKind::Tree))
            .filter_map(|c| self.to_workspace(&c.location().to_str_lossy()))
            .collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    /// Lines per author for one workspace file at HEAD (blame), largest first.
    pub fn blame(
        &self,
        path: &str,
        author_of: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Vec<(String, u32)>> {
        let repo = self.repo.to_thread_local();
        let head = repo.head_id()?.detach();
        let repo_path = if self.prefix.is_empty() {
            path.to_string()
        } else {
            format!("{}/{path}", self.prefix)
        };
        let outcome = repo.blame_file(repo_path.as_bytes().as_bstr(), head, Default::default())?;
        let mut by_author: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        let mut unknown: std::collections::HashMap<gix::ObjectId, u32> =
            std::collections::HashMap::new();
        for e in &outcome.entries {
            let hex = e.commit_id.to_string();
            match author_of(&hex) {
                Some(a) => *by_author.entry(a).or_default() += e.len.get(),
                None => *unknown.entry(e.commit_id).or_default() += e.len.get(),
            }
        }
        // Commits not mined yet (shallow history, brand-new): read their author directly.
        for (id, lines) in unknown {
            if let Ok(c) = repo.find_commit(id) {
                if let Ok(sig) = c.author() {
                    *by_author
                        .entry(sig.email.to_str_lossy().to_lowercase())
                        .or_default() += lines;
                }
            }
        }
        let mut v: Vec<(String, u32)> = by_author.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Builds a small repository with the system git (test-only; the miner itself uses gix).
    fn repo_with_history() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().ok()?;
        let p = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(p)
                .output()
                .ok()
                .filter(|o| o.status.success())
        };
        git(&["init", "-q", "-b", "main"])?;
        git(&["config", "user.email", "a@example.com"])?;
        git(&["config", "user.name", "A"])?;
        std::fs::create_dir_all(p.join("src")).ok()?;
        std::fs::write(p.join("src/a.rs"), "fn a() {}\n").ok()?;
        std::fs::write(p.join("src/b.rs"), "fn b() {}\n").ok()?;
        git(&["add", "-A"])?;
        git(&["commit", "-q", "-m", "one"])?;
        std::fs::write(p.join("src/a.rs"), "fn a() { b() }\n").ok()?;
        std::fs::write(p.join("src/b.rs"), "fn b() { }\n").ok()?;
        git(&["commit", "-q", "-am", "two"])?;
        git(&["config", "user.email", "b@example.com"])?;
        std::fs::write(p.join("README.md"), "hi\n").ok()?;
        git(&["add", "-A"])?;
        git(&["commit", "-q", "-m", "three"])?;
        Some(dir)
    }

    #[test]
    fn mines_changed_files_and_authors_incrementally() {
        let Some(dir) = repo_with_history() else {
            return;
        }; // git not installed
        let repo = Repo::open(dir.path()).unwrap();
        let all = repo.mine(None, &|_, _| {}).unwrap();
        assert_eq!(all.commits.len(), 3);
        let files: Vec<Vec<String>> = all.commits.iter().map(|c| c.files.clone()).collect();
        assert!(files.contains(&vec!["src/a.rs".to_string(), "src/b.rs".to_string()]));
        assert!(files.contains(&vec!["README.md".to_string()]));
        assert!(all.commits.iter().any(|c| c.author == "b@example.com"));
        // Nothing new since HEAD.
        let again = repo.mine(all.head.as_deref(), &|_, _| {}).unwrap();
        assert!(again.commits.is_empty());
        // Since the first commit: the two later ones.
        let first = &all.commits[2];
        let later = repo.mine(Some(&first.id), &|_, _| {}).unwrap();
        assert_eq!(later.commits.len(), 2);
        assert!(later.dropped.is_empty() && !later.replace);
        // An unknown previous head means a full re-mine.
        let unknown = repo
            .mine(Some("0123456789012345678901234567890123456789"), &|_, _| {})
            .unwrap();
        assert_eq!(unknown.commits.len(), 3);
        assert!(unknown.replace);
    }

    #[test]
    fn branch_switch_reports_dropped_commits() {
        let Some(dir) = repo_with_history() else {
            return;
        };
        let p = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(p)
                .output()
                .unwrap()
        };
        let repo = Repo::open(p).unwrap();
        let before = repo.mine(None, &|_, _| {}).unwrap();
        let old_head = before.head.clone().unwrap();
        // Rewind one commit and make a different one.
        git(&["reset", "-q", "--hard", "HEAD~1"]);
        std::fs::write(
            p.join("src/c.rs"),
            "fn c() {}
",
        )
        .unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "other"]);
        let after = repo.mine(Some(&old_head), &|_, _| {}).unwrap();
        assert_eq!(after.dropped, vec![old_head]);
        assert_eq!(after.commits.len(), 1);
        assert_eq!(after.commits[0].files, vec!["src/c.rs".to_string()]);
    }

    #[test]
    fn blames_lines_to_authors() {
        let Some(dir) = repo_with_history() else {
            return;
        };
        let repo = Repo::open(dir.path()).unwrap();
        let owners = repo.blame("src/a.rs", &|_| None).unwrap();
        assert_eq!(owners, vec![("a@example.com".to_string(), 1)]);
    }

    #[test]
    fn workspace_in_a_subdirectory_sees_relative_paths() {
        let Some(dir) = repo_with_history() else {
            return;
        };
        let repo = Repo::open(&dir.path().join("src")).unwrap();
        let all = repo.mine(None, &|_, _| {}).unwrap();
        let paths: Vec<String> = all.commits.iter().flat_map(|c| c.files.clone()).collect();
        assert!(paths.contains(&"a.rs".to_string()));
        assert!(!paths.iter().any(|p| p.contains("README")));
    }
}
