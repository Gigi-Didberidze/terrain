//! Semantic enrichment: call edges confirmed by the editor's language features.
//!
//! The extension asks VS Code's call hierarchy (backed by each language's own server) for a
//! symbol's callers and callees and sends the answers here. Confirmed calls become `semantic`
//! edges (drawn solid). Name-matched calls the language server contradicts are removed and
//! remembered as rejected, so re-resolution does not bring them back; both are forgotten when
//! the calling file's content changes, since the calls may have changed with it.

use crate::Engine;
use compass_core::model::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Confirmed {
    /// Calls the language server confirmed (new or upgraded).
    pub confirmed: u32,
    /// Name-matched calls it contradicted, now removed.
    pub rejected: u32,
}

impl Engine {
    /// Applies a call-hierarchy answer for `slot`. An empty list means "no answer" (language
    /// server missing or not ready), never "no calls", so it changes nothing.
    pub fn confirm_calls(&mut self, slot: u32, callers: &[u32], callees: &[u32]) -> Confirmed {
        let mut out = Confirmed::default();
        let Some(n) = self.graph.get(slot) else {
            return out;
        };
        if !n.kind.is_symbol() {
            return out;
        }
        let is_call = |e: &EdgeRec| e.kind == EdgeKind::Calls;
        let symbol = |g: &compass_core::Graph, s: u32| g.get(s).is_some_and(|n| n.kind.is_symbol());

        if !callees.is_empty() {
            let keep: HashSet<u32> = callees
                .iter()
                .copied()
                .filter(|&t| t != slot && symbol(&self.graph, t))
                .collect();
            let wrong: Vec<u32> = self
                .graph
                .out_edges(slot)
                .iter()
                .filter(|e| {
                    is_call(e) && e.conf == Confidence::Syntactic && !keep.contains(&e.other)
                })
                .map(|e| e.other)
                .collect();
            for &t in &wrong {
                self.reject(slot, t);
            }
            out.rejected += wrong.len() as u32;
            for &t in &keep {
                out.confirmed += self.upgrade(slot, t) as u32;
            }
        }
        if !callers.is_empty() {
            let keep: HashSet<u32> = callers
                .iter()
                .copied()
                .filter(|&c| c != slot && symbol(&self.graph, c))
                .collect();
            let wrong: Vec<u32> = self
                .graph
                .in_edges(slot)
                .iter()
                .filter(|e| {
                    is_call(e)
                        && e.conf == Confidence::Syntactic
                        && !keep.contains(&e.other)
                        && symbol(&self.graph, e.other)
                })
                .map(|e| e.other)
                .collect();
            for &c in &wrong {
                self.reject(c, slot);
            }
            out.rejected += wrong.len() as u32;
            for &c in &keep {
                out.confirmed += self.upgrade(c, slot) as u32;
            }
        }
        out
    }

    /// Replaces a name-matched call (if any) with a confirmed one. True if anything changed.
    fn upgrade(&mut self, from: u32, to: u32) -> bool {
        let already =
            self.graph.out_edges(from).iter().any(|e| {
                e.other == to && e.kind == EdgeKind::Calls && e.conf == Confidence::Semantic
            });
        if already {
            return false;
        }
        self.graph.clear_out_edges(from, |e| {
            e.other == to && e.kind == EdgeKind::Calls && e.conf == Confidence::Syntactic
        });
        self.graph
            .add_edge(from, to, EdgeKind::Calls, Confidence::Semantic, 1.0);
        true
    }

    fn reject(&mut self, from: u32, to: u32) {
        self.graph.clear_out_edges(from, |e| {
            e.other == to && e.kind == EdgeKind::Calls && e.conf == Confidence::Syntactic
        });
        let file = self.graph.node(from).file;
        let pair = (self.graph.node(from).id, self.graph.node(to).id);
        self.sem.rejected.entry(file).or_default().insert(pair);
    }

    /// The file's content changed: its calls may have too, so earlier confirmations from it
    /// no longer hold.
    pub(crate) fn forget_confirmations(&mut self, file: u32) {
        self.sem.rejected.remove(&file);
        self.graph.clear_file_out_edges(file, |e| {
            e.kind == EdgeKind::Calls && e.conf == Confidence::Semantic
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::{Engine, Options};
    use compass_core::graph::SymbolSpec;
    use compass_core::model::*;

    #[test]
    fn confirmed_calls_upgrade_and_contradicted_ones_go() {
        let mut e = Engine::new(std::path::Path::new("ws"), None, Options::default());
        let f = e.graph.upsert_file("a.rs", 10, Lang::Rust, 0);
        let spec = |name: &str| SymbolSpec {
            kind: NodeKind::Function,
            name: name.into(),
            parent: None,
            start_line: 1,
            end_line: 1,
            flags: 0,
        };
        let d = e.graph.replace_symbols(
            f,
            &[spec("run"), spec("real"), spec("wrong"), spec("caller")],
        );
        let [run, real, wrong, caller] = [d.slots[0], d.slots[1], d.slots[2], d.slots[3]];
        for t in [real, wrong] {
            e.graph
                .add_edge(run, t, EdgeKind::Calls, Confidence::Syntactic, 1.0);
        }
        let r = e.confirm_calls(run, &[caller], &[real]);
        assert_eq!(
            r,
            super::Confirmed {
                confirmed: 2,
                rejected: 1
            }
        );
        let outs: Vec<(u32, Confidence)> = e
            .graph
            .out_edges(run)
            .iter()
            .map(|x| (x.other, x.conf))
            .collect();
        assert_eq!(outs, vec![(real, Confidence::Semantic)]);
        assert!(e
            .graph
            .out_edges(caller)
            .iter()
            .any(|x| x.other == run && x.conf == Confidence::Semantic));
        assert_eq!(e.sem.rejected[&f].len(), 1);
        // An empty answer changes nothing; a repeated answer is a no-op.
        assert_eq!(e.confirm_calls(run, &[], &[]), super::Confirmed::default());
        assert_eq!(
            e.confirm_calls(run, &[caller], &[real]),
            super::Confirmed::default()
        );
        // Editing the file forgets it all.
        e.forget_confirmations(f);
        assert!(e.graph.out_edges(run).is_empty());
        assert!(!e.sem.rejected.contains_key(&f));
    }
}
