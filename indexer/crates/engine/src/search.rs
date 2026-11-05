//! Fuzzy search over file and symbol names. Budget: < 30 ms per keystroke on linux.
//! A 64-bit character mask rejects most entries before any scoring; scoring rewards matches at
//! word boundaries and runs of consecutive characters. Ties are broken by a per-node boost
//! (load-bearing score once metrics exist).

use rayon::prelude::*;
use std::collections::HashMap;

struct Entry {
    slot: u32,
    /// Lowercased name (file name or symbol name).
    name: Box<[u8]>,
    /// Original-case name, for camelCase boundaries.
    orig: Box<[u8]>,
    /// Lowercased container (path for files, file path for symbols), searched as a fallback.
    context: Box<[u8]>,
    mask: u64,
}

#[derive(Default)]
pub struct SearchIndex {
    entries: Vec<Entry>,
    pos: HashMap<u32, usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub slot: u32,
    pub score: f32,
}

fn char_mask(s: &[u8]) -> u64 {
    let mut m = 0u64;
    for &b in s {
        m |= 1 << bit(b);
    }
    m
}

fn bit(b: u8) -> u32 {
    match b.to_ascii_lowercase() {
        c @ b'a'..=b'z' => (c - b'a') as u32,
        c @ b'0'..=b'9' => 26 + (c - b'0') as u32,
        b'_' => 36,
        b'.' => 37,
        b'/' => 38,
        b'-' => 39,
        _ => 40 + (b as u32 % 24),
    }
}

impl SearchIndex {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn upsert(&mut self, slot: u32, name: &str, context: &str) {
        let lower: Box<[u8]> = name.to_ascii_lowercase().into_bytes().into();
        let mask = char_mask(&lower) | char_mask(context.as_bytes());
        let e = Entry {
            slot,
            mask,
            orig: name.as_bytes().into(),
            name: lower,
            context: context.to_ascii_lowercase().into_bytes().into(),
        };
        match self.pos.get(&slot) {
            Some(&i) => self.entries[i] = e,
            None => {
                self.pos.insert(slot, self.entries.len());
                self.entries.push(e);
            }
        }
    }

    pub fn remove(&mut self, slot: u32) {
        if let Some(i) = self.pos.remove(&slot) {
            self.entries.swap_remove(i);
            if i < self.entries.len() {
                self.pos.insert(self.entries[i].slot, i);
            }
        }
    }

    pub fn search(&self, query: &str, limit: usize, boost: impl Fn(u32) -> f32 + Sync) -> Vec<Hit> {
        let q: Vec<u8> = query
            .trim()
            .to_ascii_lowercase()
            .into_bytes()
            .into_iter()
            .filter(|b| *b != b' ')
            .collect();
        if q.is_empty() {
            return Vec::new();
        }
        let qmask = char_mask(&q);
        let has_slash = q.contains(&b'/');
        let mut hits: Vec<Hit> = self
            .entries
            .par_iter()
            .filter(|e| e.mask & qmask == qmask)
            .filter_map(|e| {
                let s = if has_slash {
                    None
                } else {
                    score(&q, &e.name, &e.orig)
                };
                let s = s.or_else(|| {
                    // Fall back to the full path: "vs/base/arr" finds src/vs/base/common/arrays.ts.
                    let mut full = e.context.to_vec();
                    if !full.is_empty() && !full.ends_with(b"/") && !e.context.ends_with(&e.name) {
                        full.push(b'/');
                        full.extend_from_slice(&e.name);
                    }
                    score(&q, &full, &full).map(|s| s * 0.6)
                })?;
                Some(Hit {
                    slot: e.slot,
                    score: s + boost(e.slot),
                })
            })
            .collect();
        hits.par_sort_unstable_by(|a, b| b.score.total_cmp(&a.score).then(a.slot.cmp(&b.slot)));
        hits.truncate(limit);
        hits
    }
}

fn is_boundary(orig: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = orig[i - 1];
    let cur = orig[i];
    matches!(prev, b'/' | b'_' | b'-' | b'.' | b' ' | b':')
        || (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
}

/// Scores `q` as a subsequence of `s`. Tries a few start positions and keeps the best.
pub fn score(q: &[u8], s: &[u8], orig: &[u8]) -> Option<f32> {
    if q.len() > s.len() {
        return None;
    }
    if q == s {
        return Some(1000.0);
    }
    let mut best: Option<f32> = None;
    let mut starts = 0;
    for start in 0..s.len() {
        if s[start] != q[0] {
            continue;
        }
        if starts >= 8 {
            break;
        }
        starts += 1;
        let mut sc = 0.0f32;
        let mut qi = 0;
        let mut last: Option<usize> = None;
        let mut i = start;
        while i < s.len() && qi < q.len() {
            if s[i] == q[qi] {
                sc += 16.0;
                if is_boundary(orig, i) {
                    sc += 12.0;
                }
                match last {
                    Some(l) if l + 1 == i => sc += 10.0,
                    Some(l) => sc -= ((i - l - 1) as f32).min(8.0),
                    None => {}
                }
                last = Some(i);
                qi += 1;
            }
            i += 1;
        }
        if qi < q.len() {
            break; // Later starts can't match either.
        }
        if start == 0 {
            sc += 20.0;
        }
        // Prefer shorter names: "map" ranks map.rs above mapping_utilities.rs.
        sc -= (s.len() as f32 - q.len() as f32) * 0.25;
        if s.starts_with(q) {
            sc += 40.0;
        }
        best = Some(best.map_or(sc, |b: f32| b.max(sc)));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> SearchIndex {
        let mut ix = SearchIndex::default();
        ix.upsert(1, "arrays.ts", "src/vs/base/common/arrays.ts");
        ix.upsert(2, "arraysFind.ts", "src/vs/base/common/arraysFind.ts");
        ix.upsert(3, "QuerySet", "django/db/models/query.py");
        ix.upsert(4, "query.py", "django/db/models/query.py");
        ix.upsert(5, "parse_query_string", "x/y.py");
        ix
    }

    #[test]
    fn ranks_exact_and_prefix_matches_first() {
        let ix = index();
        let hits = ix.search("arrays", 10, |_| 0.0);
        assert_eq!(hits[0].slot, 1);
        assert_eq!(hits[1].slot, 2);
    }

    #[test]
    fn matches_camel_case_boundaries() {
        let ix = index();
        let hits = ix.search("qs", 10, |_| 0.0);
        assert_eq!(hits[0].slot, 3);
    }

    #[test]
    fn falls_back_to_paths() {
        let ix = index();
        let hits = ix.search("vs/base/arrf", 10, |_| 0.0);
        assert_eq!(hits.first().map(|h| h.slot), Some(2));
    }

    #[test]
    fn remove_keeps_positions_consistent() {
        let mut ix = index();
        ix.remove(1);
        ix.remove(5);
        ix.upsert(2, "renamed.ts", "src/renamed.ts");
        assert!(ix.search("arrays", 10, |_| 0.0).iter().all(|h| h.slot != 1));
        assert_eq!(ix.search("renamed", 10, |_| 0.0)[0].slot, 2);
        assert_eq!(ix.len(), 3);
    }
}
