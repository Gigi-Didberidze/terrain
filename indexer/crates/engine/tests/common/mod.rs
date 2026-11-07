//! Shared test helpers: graph dumps that identify nodes by path and qualified name, so two
//! independently built graphs can be compared.
#![allow(dead_code)]

use terrain_core::model::*;
use terrain_engine::{Engine, Shared};

pub fn id(e: &Engine, slot: u32) -> String {
    let n = e.graph.node(slot);
    if n.kind.is_symbol() {
        format!(
            "{}::{}",
            e.graph.path(n.file).unwrap_or("?"),
            e.graph.qualified_name(slot)
        )
    } else {
        e.graph.path(slot).unwrap_or("?").to_string()
    }
}

/// Nodes and edges as sorted lines.
pub fn dump(s: &Shared) -> (Vec<String>, Vec<String>) {
    let e = s.read();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (slot, n) in e.graph.nodes.iter().enumerate() {
        let slot = slot as u32;
        // The golden file itself lives in the fixture tree; it is not part of the fixture.
        let is_golden = n.kind == NodeKind::File && e.graph.path(slot) == Some("expected.json");
        if !n.alive() || n.kind == NodeKind::Dir || is_golden {
            continue;
        }
        let mut tags = Vec::new();
        if n.flags & flags::EXPORTED != 0 {
            tags.push("exported");
        }
        if n.flags & flags::ENTRY != 0 {
            tags.push("entry");
        }
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(","))
        };
        nodes.push(format!("{} {}{}", n.kind.as_str(), id(&e, slot), tags));
        for ed in e.graph.out_edges(slot) {
            let w = if (ed.weight - 1.0).abs() > 1e-6 {
                format!(" ({:.2})", ed.weight)
            } else {
                String::new()
            };
            edges.push(format!(
                "{} -{}-> {}{}",
                id(&e, slot),
                ed.kind.as_str(),
                id(&e, ed.other),
                w
            ));
        }
    }
    nodes.sort();
    edges.sort();
    (nodes, edges)
}
