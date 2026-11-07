//! Graph algorithms behind the meaning layer: PageRank (load-bearing score), Tarjan strongly
//! connected components (cycles) with the edge whose removal breaks most of each cycle, Leiden
//! community detection, and bounded reachability (blast radius).
//!
//! Graphs are compressed sparse rows over dense indices `0..n` so they stay small at linux scale.

/// Compressed sparse row adjacency.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Csr {
    pub offsets: Vec<u32>,
    pub targets: Vec<u32>,
    pub weights: Vec<f32>,
}

impl Csr {
    /// Builds from (source, target, weight) triples. Parallel edges are summed, self-loops dropped.
    pub fn from_edges(n: usize, mut edges: Vec<(u32, u32, f32)>) -> Csr {
        edges.retain(|e| e.0 != e.1);
        edges.sort_unstable_by_key(|e| (e.0, e.1));
        let mut offsets = vec![0u32; n + 1];
        let mut targets = Vec::with_capacity(edges.len());
        let mut weights = Vec::with_capacity(edges.len());
        let mut last: Option<(u32, u32)> = None;
        for (s, t, w) in edges {
            if last == Some((s, t)) {
                *weights.last_mut().unwrap() += w;
                continue;
            }
            last = Some((s, t));
            offsets[s as usize + 1] += 1;
            targets.push(t);
            weights.push(w);
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        Csr {
            offsets,
            targets,
            weights,
        }
    }

    pub fn n(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn neighbors(&self, i: u32) -> impl Iterator<Item = (u32, f32)> + '_ {
        let (a, b) = (
            self.offsets[i as usize] as usize,
            self.offsets[i as usize + 1] as usize,
        );
        self.targets[a..b]
            .iter()
            .copied()
            .zip(self.weights[a..b].iter().copied())
    }

    pub fn degree(&self, i: u32) -> usize {
        (self.offsets[i as usize + 1] - self.offsets[i as usize]) as usize
    }

    pub fn reversed(&self) -> Csr {
        let n = self.n();
        let mut edges = Vec::with_capacity(self.targets.len());
        for s in 0..n as u32 {
            for (t, w) in self.neighbors(s) {
                edges.push((t, s, w));
            }
        }
        Csr::from_edges(n, edges)
    }

    /// Undirected version (weights of both directions summed), for community detection.
    pub fn symmetric(&self) -> Csr {
        let n = self.n();
        let mut edges = Vec::with_capacity(self.targets.len() * 2);
        for s in 0..n as u32 {
            for (t, w) in self.neighbors(s) {
                edges.push((s, t, w));
                edges.push((t, s, w));
            }
        }
        Csr::from_edges(n, edges)
    }
}

/// PageRank where rank flows along dependency edges (A → B: A depends on B), so a node scores
/// high when many (important) things rest on it. Weighted; dangling rank is spread uniformly.
pub fn pagerank(g: &Csr, damping: f64, iterations: usize) -> Vec<f64> {
    let n = g.n();
    if n == 0 {
        return Vec::new();
    }
    let out_w: Vec<f64> = (0..n as u32)
        .map(|i| g.neighbors(i).map(|(_, w)| w as f64).sum())
        .collect();
    let mut rank = vec![1.0 / n as f64; n];
    let mut next = vec![0.0; n];
    for _ in 0..iterations {
        let dangling: f64 = (0..n).filter(|&i| out_w[i] == 0.0).map(|i| rank[i]).sum();
        let base = (1.0 - damping) / n as f64 + damping * dangling / n as f64;
        next.iter_mut().for_each(|x| *x = base);
        for s in 0..n {
            if out_w[s] == 0.0 {
                continue;
            }
            let share = damping * rank[s] / out_w[s];
            for (t, w) in g.neighbors(s as u32) {
                next[t as usize] += share * w as f64;
            }
        }
        let delta: f64 = rank.iter().zip(&next).map(|(a, b)| (a - b).abs()).sum();
        std::mem::swap(&mut rank, &mut next);
        if delta < 1e-9 {
            break;
        }
    }
    rank
}

/// Tarjan's strongly connected components (iterative). Returns components with more than one
/// node, each sorted, largest first.
pub fn strongly_connected(g: &Csr) -> Vec<Vec<u32>> {
    let n = g.n();
    const UNSEEN: u32 = u32::MAX;
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    let mut counter = 0u32;
    // Explicit call stack: (node, next neighbour position).
    let mut call: Vec<(u32, usize)> = Vec::new();
    for root in 0..n as u32 {
        if index[root as usize] != UNSEEN {
            continue;
        }
        call.push((root, g.offsets[root as usize] as usize));
        index[root as usize] = counter;
        low[root as usize] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        while let Some(&mut (v, ref mut pos)) = call.last_mut() {
            let end = g.offsets[v as usize + 1] as usize;
            if *pos < end {
                let w = g.targets[*pos];
                *pos += 1;
                if index[w as usize] == UNSEEN {
                    index[w as usize] = counter;
                    low[w as usize] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w as usize] = true;
                    call.push((w, g.offsets[w as usize] as usize));
                } else if on_stack[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent as usize] = low[parent as usize].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                let mut comp = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on_stack[w as usize] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                if comp.len() > 1 {
                    comp.sort_unstable();
                    out.push(comp);
                }
            }
        }
    }
    out.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    out
}

/// The edge inside a cycle whose removal leaves the smallest remaining cycle (largest SCC within
/// `comp`). Exact for components up to `exact_limit` edges; above that, a heuristic picks the
/// lightest edge into the node with the most in-cycle dependents.
pub fn best_break_edge(g: &Csr, comp: &[u32], exact_limit: usize) -> Option<(u32, u32)> {
    let pos: std::collections::HashMap<u32, u32> = comp
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i as u32))
        .collect();
    let mut inner: Vec<(u32, u32, f32)> = Vec::new();
    for &v in comp {
        for (t, w) in g.neighbors(v) {
            if let Some(&ti) = pos.get(&t) {
                inner.push((pos[&v], ti, w));
            }
        }
    }
    if inner.is_empty() {
        return None;
    }
    if inner.len() > exact_limit {
        let mut indeg = vec![0u32; comp.len()];
        for e in &inner {
            indeg[e.1 as usize] += 1;
        }
        let hub = (0..comp.len()).max_by_key(|&i| (indeg[i], std::cmp::Reverse(i)))? as u32;
        let e = inner
            .iter()
            .filter(|e| e.1 == hub)
            .min_by(|a, b| a.2.total_cmp(&b.2).then((a.0, a.1).cmp(&(b.0, b.1))))?;
        return Some((comp[e.0 as usize], comp[e.1 as usize]));
    }
    let mut best: Option<(usize, f32, (u32, u32))> = None;
    for (i, e) in inner.iter().enumerate() {
        let rest: Vec<(u32, u32, f32)> = inner
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, e)| *e)
            .collect();
        let sub = Csr::from_edges(comp.len(), rest);
        let largest = strongly_connected(&sub)
            .first()
            .map(|c| c.len())
            .unwrap_or(0);
        let cand = (largest, e.2, (e.0, e.1));
        // Smallest remaining cycle wins; ties go to the lighter edge (least used dependency).
        if best.is_none_or(|b| (cand.0, cand.1) < (b.0, b.1)) {
            best = Some(cand);
        }
    }
    best.map(|(_, _, (a, b))| (comp[a as usize], comp[b as usize]))
}

/// Nodes reachable from `start` along `g` within `max_depth` steps (excluding `start`), with
/// their depth, in breadth-first order. With `g` = reversed dependencies this is the blast
/// radius: everything that (transitively) depends on `start`.
pub fn reachable(g: &Csr, start: u32, max_depth: u32, limit: usize) -> Vec<(u32, u32)> {
    let mut seen = std::collections::HashSet::new();
    seen.insert(start);
    let mut out = Vec::new();
    let mut frontier = vec![start];
    for depth in 1..=max_depth {
        let mut next = Vec::new();
        for &v in &frontier {
            for (t, _) in g.neighbors(v) {
                if seen.insert(t) {
                    out.push((t, depth));
                    next.push(t);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

/// Leiden community detection (Traag, Waltman & van Eck 2019) on an undirected weighted graph,
/// maximising modularity with resolution `gamma`. Deterministic: nodes are visited in index
/// order and the refinement merges greedily (the θ → 0 limit of the randomised step), so the
/// same graph always gives the same communities. Returns a community id per node.
pub fn leiden(g: &Csr, gamma: f64, max_levels: usize) -> Vec<u32> {
    let n = g.n();
    if n == 0 {
        return Vec::new();
    }
    // Working graph: adjacency lists with self-loops allowed, explicit node degrees.
    let mut adj: Vec<Vec<(u32, f64)>> = (0..n as u32)
        .map(|i| g.neighbors(i).map(|(t, w)| (t, w as f64)).collect())
        .collect();
    let mut deg: Vec<f64> = adj.iter().map(|a| a.iter().map(|e| e.1).sum()).collect();
    let two_m: f64 = deg.iter().sum();
    if two_m == 0.0 {
        return (0..n as u32).collect();
    }
    // membership[original node] = node in the current aggregate graph.
    let mut membership: Vec<u32> = (0..n as u32).collect();
    let mut part: Vec<u32> = (0..n as u32).collect();
    for _ in 0..max_levels {
        let size = adj.len();
        move_nodes_fast(&adj, &deg, &mut part, gamma, two_m);
        let communities = renumber(&mut part);
        if communities == size {
            break;
        }
        let refined = refine(&adj, &deg, &part, gamma, two_m);
        let mut refined = refined;
        let rc = renumber(&mut refined);
        // Aggregate on the refined partition; start from the unrefined one.
        let mut new_adj: Vec<std::collections::HashMap<u32, f64>> =
            vec![std::collections::HashMap::new(); rc];
        let mut new_deg = vec![0.0; rc];
        for v in 0..size {
            let rv = refined[v];
            new_deg[rv as usize] += deg[v];
            for &(t, w) in &adj[v] {
                *new_adj[rv as usize]
                    .entry(refined[t as usize])
                    .or_insert(0.0) += w;
            }
        }
        let mut new_part = vec![0u32; rc];
        for v in 0..size {
            new_part[refined[v] as usize] = part[v];
        }
        for m in membership.iter_mut() {
            *m = refined[*m as usize];
        }
        adj = new_adj
            .into_iter()
            .map(|h| {
                let mut v: Vec<(u32, f64)> = h.into_iter().collect();
                v.sort_by_key(|e| e.0);
                v
            })
            .collect();
        deg = new_deg;
        part = new_part;
        if rc == size {
            break;
        }
    }
    let mut result: Vec<u32> = membership.iter().map(|&m| part[m as usize]).collect();
    renumber(&mut result);
    result
}

/// Renumbers community ids to 0..k in order of first appearance; returns k.
fn renumber(part: &mut [u32]) -> usize {
    let mut map = std::collections::HashMap::new();
    for p in part.iter_mut() {
        let next = map.len() as u32;
        *p = *map.entry(*p).or_insert(next);
    }
    map.len()
}

fn move_nodes_fast(adj: &[Vec<(u32, f64)>], deg: &[f64], part: &mut [u32], gamma: f64, two_m: f64) {
    let n = adj.len();
    let mut tot = vec![0.0; n.max(part.iter().map(|&p| p as usize + 1).max().unwrap_or(0))];
    for v in 0..n {
        tot[part[v] as usize] += deg[v];
    }
    let mut queue: std::collections::VecDeque<u32> = (0..n as u32).collect();
    let mut queued = vec![true; n];
    let mut links: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
    let mut empty_pool: Vec<u32> = Vec::new();
    while let Some(v) = queue.pop_front() {
        let v = v as usize;
        queued[v] = false;
        let old = part[v];
        links.clear();
        for &(t, w) in &adj[v] {
            if t as usize != v {
                *links.entry(part[t as usize]).or_insert(0.0) += w;
            }
        }
        tot[old as usize] -= deg[v];
        if tot[old as usize] <= 0.0 {
            empty_pool.push(old);
        }
        let gain = |c: u32, k_in: f64, tot: &[f64]| k_in - gamma * deg[v] * tot[c as usize] / two_m;
        let mut best = old;
        let mut best_gain = gain(old, links.get(&old).copied().unwrap_or(0.0), &tot);
        let mut cands: Vec<(u32, f64)> = links.iter().map(|(&c, &k)| (c, k)).collect();
        cands.sort_by_key(|c| c.0);
        for (c, k) in cands {
            let g = gain(c, k, &tot);
            if g > best_gain + 1e-12 {
                best = c;
                best_gain = g;
            }
        }
        // Moving to an empty community scores 0.
        if best_gain < -1e-12 {
            if let Some(e) = empty_pool.iter().copied().find(|&e| tot[e as usize] <= 0.0) {
                best = e;
            }
        }
        part[v] = best;
        tot[best as usize] += deg[v];
        if best != old {
            for &(t, _) in &adj[v] {
                let t = t as usize;
                if t != v && part[t] != best && !queued[t] {
                    queued[t] = true;
                    queue.push_back(t as u32);
                }
            }
        }
    }
}

/// Leiden refinement: inside each community, start from singletons and merge well-connected
/// nodes into well-connected sub-communities, greedily. Guarantees connected communities.
fn refine(adj: &[Vec<(u32, f64)>], deg: &[f64], part: &[u32], gamma: f64, two_m: f64) -> Vec<u32> {
    let n = adj.len();
    let mut refined: Vec<u32> = (0..n as u32).collect();
    let mut sub_tot: Vec<f64> = deg.to_vec();
    let mut singleton = vec![true; n];
    let k = part.iter().map(|&p| p as usize + 1).max().unwrap_or(0);
    let mut comm_tot = vec![0.0; k];
    for v in 0..n {
        comm_tot[part[v] as usize] += deg[v];
    }
    // Edge weight from each node to the rest of its community.
    let to_comm: Vec<f64> = (0..n)
        .map(|v| {
            adj[v]
                .iter()
                .filter(|(t, _)| *t as usize != v && part[*t as usize] == part[v])
                .map(|e| e.1)
                .sum()
        })
        .collect();
    // Weight from each refined sub-community to the rest of its community.
    let mut sub_to_comm: Vec<f64> = to_comm.clone();
    let mut links: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
    for v in 0..n {
        if !singleton[v] {
            continue;
        }
        let c = part[v] as usize;
        // Only well-connected nodes move.
        if to_comm[v] < gamma * deg[v] * (comm_tot[c] - deg[v]) / two_m {
            continue;
        }
        links.clear();
        for &(t, w) in &adj[v] {
            let t = t as usize;
            if t != v && part[t] as usize == c {
                *links.entry(refined[t]).or_insert(0.0) += w;
            }
        }
        let mut best: Option<(u32, f64)> = None;
        let mut cands: Vec<(u32, f64)> = links.iter().map(|(&s, &w)| (s, w)).collect();
        cands.sort_by_key(|c| c.0);
        for (s, k_in) in cands {
            if s == refined[v] {
                continue;
            }
            // Target sub-community must itself be well connected to its community.
            let st = sub_tot[s as usize];
            if sub_to_comm[s as usize] < gamma * st * (comm_tot[c] - st) / two_m {
                continue;
            }
            let gain = k_in - gamma * deg[v] * st / two_m;
            if gain >= 0.0 && best.is_none_or(|b| gain > b.1 + 1e-12) {
                best = Some((s, gain));
            }
        }
        if let Some((s, _)) = best {
            let own = refined[v];
            sub_tot[own as usize] -= deg[v];
            refined[v] = s;
            sub_tot[s as usize] += deg[v];
            singleton[v] = false;
            // Update the sub-community's connection to the rest of the community.
            let w_to_s: f64 = adj[v]
                .iter()
                .filter(|(t, _)| refined[*t as usize] == s && *t as usize != v)
                .map(|e| e.1)
                .sum();
            sub_to_comm[s as usize] += to_comm[v] - 2.0 * w_to_s;
            for &(t, _) in &adj[v] {
                if refined[t as usize] == s {
                    singleton[t as usize] = false;
                }
            }
        }
    }
    refined
}

/// Modularity of a partition (for tests and diagnostics).
pub fn modularity(g: &Csr, part: &[u32], gamma: f64) -> f64 {
    let n = g.n();
    let deg: Vec<f64> = (0..n as u32)
        .map(|i| g.neighbors(i).map(|(_, w)| w as f64).sum())
        .collect();
    let two_m: f64 = deg.iter().sum();
    if two_m == 0.0 {
        return 0.0;
    }
    let k = part.iter().map(|&p| p as usize + 1).max().unwrap_or(0);
    let mut inside = vec![0.0; k];
    let mut tot = vec![0.0; k];
    for v in 0..n {
        tot[part[v] as usize] += deg[v];
        for (t, w) in g.neighbors(v as u32) {
            if part[t as usize] == part[v] {
                inside[part[v] as usize] += w as f64;
            }
        }
    }
    (0..k)
        .map(|c| inside[c] / two_m - gamma * (tot[c] / two_m).powi(2))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(n: usize, e: &[(u32, u32)]) -> Csr {
        Csr::from_edges(n, e.iter().map(|&(a, b)| (a, b, 1.0)).collect())
    }

    #[test]
    fn pagerank_rewards_what_many_depend_on() {
        // 0,1,2,3 all depend on 4; 4 depends on 5.
        let r = pagerank(&g(6, &[(0, 4), (1, 4), (2, 4), (3, 4), (4, 5)]), 0.85, 100);
        let order: Vec<usize> = {
            let mut i: Vec<usize> = (0..6).collect();
            i.sort_by(|a, b| r[*b].total_cmp(&r[*a]));
            i
        };
        assert_eq!(&order[..2], &[5, 4], "what everything rests on ranks first");
        assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn finds_cycles_and_the_edge_that_breaks_them() {
        // Cycle 0→1→2→0 plus a back edge 2→1 (inner cycle 1⇄2); 3 is outside.
        let graph = g(4, &[(0, 1), (1, 2), (2, 0), (2, 1), (2, 3)]);
        let sccs = strongly_connected(&graph);
        assert_eq!(sccs, vec![vec![0, 1, 2]]);
        // Removing 1→2 breaks both cycles at once.
        assert_eq!(best_break_edge(&graph, &sccs[0], 1000), Some((1, 2)));
    }

    #[test]
    fn scc_handles_deep_chains_without_recursion() {
        let n = 200_000u32;
        let mut e: Vec<(u32, u32)> = (0..n - 1).map(|i| (i, i + 1)).collect();
        e.push((n - 1, 0));
        let sccs = strongly_connected(&g(n as usize, &e));
        assert_eq!(sccs.len(), 1);
        assert_eq!(sccs[0].len(), n as usize);
    }

    #[test]
    fn blast_radius_is_bounded_by_depth() {
        // 1 depends on 0, 2 on 1, 3 on 2: dependents of 0 are 1, 2, 3 at depths 1..3.
        let dependents = g(4, &[(1, 0), (2, 1), (3, 2)]).reversed();
        assert_eq!(
            reachable(&dependents, 0, 6, 100),
            vec![(1, 1), (2, 2), (3, 3)]
        );
        assert_eq!(reachable(&dependents, 0, 2, 100), vec![(1, 1), (2, 2)]);
    }

    #[test]
    fn leiden_finds_planted_communities() {
        // Three cliques of 6 joined by single bridges.
        let mut e = Vec::new();
        for c in 0..3u32 {
            for a in 0..6u32 {
                for b in a + 1..6 {
                    e.push((c * 6 + a, c * 6 + b));
                }
            }
        }
        e.push((5, 6));
        e.push((11, 12));
        let graph = g(18, &e).symmetric();
        let part = leiden(&graph, 1.0, 10);
        for c in 0..3 {
            let first = part[c * 6];
            assert!(
                (0..6).all(|i| part[c * 6 + i] == first),
                "clique {c} split: {part:?}"
            );
        }
        assert_ne!(part[0], part[6]);
        assert_ne!(part[6], part[12]);
        assert!(modularity(&graph, &part, 1.0) > 0.5);
        // Deterministic.
        assert_eq!(part, leiden(&graph, 1.0, 10));
    }
}
