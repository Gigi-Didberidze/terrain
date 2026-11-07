//! Runs a language's queries over one file and produces [`FileFacts`].

use crate::facts::*;
use crate::langs::{spec, Spec};
use compass_core::model::{Lang, NodeKind};
use std::cell::RefCell;
use std::collections::HashMap;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};

thread_local! {
    /// One parser per thread (parsers are not Sync); reused across files.
    static PARSERS: RefCell<HashMap<Lang, Parser>> = RefCell::new(HashMap::new());
}

pub fn parse(lang: Lang, src: &[u8]) -> Option<Tree> {
    let spec = spec(lang)?;
    PARSERS.with(|p| {
        let mut map = p.borrow_mut();
        let parser = map.entry(lang).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&spec.language)
                .expect("grammar version compatible");
            p
        });
        parser.parse(src, None)
    })
}

/// Extracts definitions, references, imports and entry points. None if the language has no
/// grammar (Tier 3) or parsing failed.
pub fn extract(lang: Lang, src: &[u8]) -> Option<FileFacts> {
    let spec = spec(lang)?;
    let tree = parse(lang, src)?;
    Some(extract_tree(spec, &tree, src))
}

struct RawDef {
    kind: NodeKind,
    name: String,
    start: usize,
    end: usize,
    name_start: usize,
    start_line: u32,
    end_line: u32,
    exported: bool,
    /// Type a method belongs to without being nested in it (Go receiver).
    owner: Option<String>,
    /// A Rust `impl` block: merged into the type of the same name when the file defines it.
    impl_block: bool,
}

/// Lower is more specific; used when one node matches several definition patterns.
fn kind_rank(k: NodeKind) -> u8 {
    match k {
        NodeKind::Method => 0,
        NodeKind::Function => 1,
        NodeKind::Class => 2,
        NodeKind::Type => 3,
        NodeKind::Module => 4,
        _ => 5,
    }
}

fn def_kind(capture: &str) -> Option<NodeKind> {
    let k = capture
        .strip_prefix("def.")
        .or_else(|| capture.strip_prefix("definition."))?;
    Some(match k {
        "function" | "macro" => NodeKind::Function,
        "method" => NodeKind::Method,
        "class" | "struct" | "enum" => NodeKind::Class,
        "type" | "interface" | "trait" => NodeKind::Type,
        "constant" | "variable" => NodeKind::Constant,
        "module" | "namespace" => NodeKind::Module,
        _ => return None,
    })
}

fn ref_kind(capture: &str) -> Option<RefKind> {
    let k = capture
        .strip_prefix("ref.")
        .or_else(|| capture.strip_prefix("reference."))?;
    Some(match k {
        "call" | "class" => RefKind::Call,
        "type" => RefKind::Type,
        "inherits" => RefKind::Inherits,
        "implements" | "implementation" => RefKind::Implements,
        _ => return None,
    })
}

fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[n.byte_range()]).unwrap_or("")
}

fn line(n: Node) -> u32 {
    n.start_position().row as u32 + 1
}

/// One query match: (capture name, node) pairs.
type Match<'q, 't> = Vec<(&'q str, Node<'t>)>;

fn collect<'q, 't>(q: &'q Query, root: Node<'t>, src: &[u8]) -> Vec<(usize, Match<'q, 't>)> {
    let names = q.capture_names();
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut matches = cursor.matches(q, root, src);
    while let Some(m) = matches.next() {
        out.push((
            m.pattern_index,
            m.captures()
                .iter()
                .map(|c| (names[c.index as usize], c.node))
                .collect(),
        ));
    }
    out
}

/// Matches per section (defs, refs, imports, entries): one combined walk when available.
fn sections<'q, 't>(spec: &'q Spec, root: Node<'t>, src: &[u8]) -> [Vec<Match<'q, 't>>; 4] {
    let mut out: [Vec<Match>; 4] = Default::default();
    if let Some((q, bounds)) = &spec.combined {
        for (pattern, m) in collect(q, root, src) {
            let section = (0..4).find(|&i| pattern < bounds[i + 1]).unwrap_or(3);
            out[section].push(m);
        }
        return out;
    }
    let qs = [
        Some(&spec.defs),
        Some(&spec.refs),
        spec.imports.as_ref(),
        spec.entries.as_ref(),
    ];
    for (i, q) in qs.into_iter().enumerate() {
        if let Some(q) = q {
            out[i] = collect(q, root, src).into_iter().map(|(_, m)| m).collect();
        }
    }
    out
}

pub fn extract_tree(spec: &Spec, tree: &Tree, src: &[u8]) -> FileFacts {
    let root = tree.root_node();
    let mut facts = FileFacts::default();
    let [def_matches, ref_matches, import_matches, entry_matches] = sections(spec, root, src);

    // ---- definitions
    let mut raw: Vec<RawDef> = Vec::new();
    for m in &def_matches {
        let mut def: Option<(NodeKind, Node)> = None;
        let mut name: Option<Node> = None;
        let mut owner: Option<String> = None;
        let mut impl_block = false;
        for &(cn, node) in m {
            if cn == "name" {
                name = Some(node);
            } else if cn == "owner" {
                owner = Some(text(node, src).to_string());
            } else if cn == "def.impl" {
                def = Some((NodeKind::Class, node));
                impl_block = true;
            } else if cn == "namespace" {
                facts.namespaces.push(text(node, src).to_string());
            } else if let Some(k) = def_kind(cn) {
                def = Some((k, node));
            }
        }
        let (Some((kind, node)), Some(name)) = (def, name) else {
            continue;
        };
        let name_s = text(name, src);
        if name_s.is_empty() {
            continue;
        }
        raw.push(RawDef {
            kind,
            name: name_s.to_string(),
            start: node.start_byte(),
            end: node.end_byte(),
            name_start: name.start_byte(),
            start_line: line(node),
            end_line: node.end_position().row as u32 + 1,
            exported: exported(spec.lang, node, name_s, src),
            owner,
            impl_block,
        });
    }
    // Same node matched by several patterns: keep the most specific kind.
    raw.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.name_start.cmp(&b.name_start))
            .then(kind_rank(a.kind).cmp(&kind_rank(b.kind)))
    });
    raw.dedup_by(|b, a| {
        let same = a.start == b.start && a.end == b.end && a.name_start == b.name_start;
        if same && a.owner.is_none() {
            a.owner = b.owner.take();
        }
        same
    });

    // Nesting by containment.
    let mut stack: Vec<usize> = Vec::new();
    for (i, d) in raw.iter().enumerate() {
        while let Some(&top) = stack.last() {
            if raw[top].end >= d.end && raw[top].start <= d.start {
                break;
            }
            stack.pop();
        }
        let parent = stack.last().copied();
        let mut kind = d.kind;
        if let Some(p) = parent {
            // A function directly inside a class or type is a method.
            if kind == NodeKind::Function
                && raw[p].kind.is_container()
                && raw[p].kind != NodeKind::Module
            {
                kind = NodeKind::Method;
            }
        } else if kind == NodeKind::Method {
            // Methods outside any container (Go, Rust impls captured without the impl) stay methods.
        }
        facts.defs.push(Def {
            name: d.name.clone(),
            kind,
            start_line: d.start_line,
            end_line: d.end_line,
            parent: parent.map(|p| p as u32),
            exported: d.exported,
            entry: false,
        });
        stack.push(i);
    }
    let def_at_name: HashMap<usize, usize> = raw
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name_start, i))
        .collect();
    let enclosing = |pos: usize| -> Option<u32> {
        // Last def starting at or before pos, then walk up until one contains pos.
        let mut i = match raw.binary_search_by(|d| d.start.cmp(&pos)) {
            Ok(i) => {
                let mut j = i;
                while j + 1 < raw.len() && raw[j + 1].start == pos {
                    j += 1;
                }
                j
            }
            Err(0) => return None,
            Err(i) => i - 1,
        };
        loop {
            if raw[i].start <= pos && pos < raw[i].end {
                return Some(i as u32);
            }
            i = facts.defs[i].parent? as usize;
        }
    };

    // ---- references
    let mut seen = std::collections::HashSet::new();
    let mut ref_pos: Vec<usize> = Vec::new();
    for m in &ref_matches {
        let mut kind: Option<RefKind> = None;
        let mut target: Option<Node> = None;
        let mut tagged_name: Option<Node> = None;
        for &(cn, node) in m {
            if cn == "name" {
                tagged_name = Some(node);
            } else if let Some(k) = ref_kind(cn) {
                kind = Some(k);
                target = Some(node);
            }
        }
        let Some(kind) = kind else { continue };
        // Tier 2 tags put the kind on the whole expression and the name in @name.
        let node = tagged_name.or(target).unwrap();
        if def_at_name.contains_key(&node.start_byte()) {
            continue; // A definition's own name, not a use.
        }
        if !seen.insert((node.start_byte(), kind)) {
            continue;
        }
        let name = text(node, src);
        if name.is_empty() || name.len() > 200 {
            continue;
        }
        facts.refs.push(Ref {
            name: name.to_string(),
            kind,
            line: line(node),
            from: enclosing(node.start_byte()),
        });
        ref_pos.push(node.start_byte());
    }
    // A name captured as inherits/implements is not also a plain type reference.
    let specific: std::collections::HashSet<usize> = facts
        .refs
        .iter()
        .zip(&ref_pos)
        .filter(|(r, _)| matches!(r.kind, RefKind::Inherits | RefKind::Implements))
        .map(|(_, &p)| p)
        .collect();
    if !specific.is_empty() {
        let keep: Vec<bool> = ref_pos
            .iter()
            .zip(&facts.refs)
            .map(|(p, r)| !(r.kind == RefKind::Type && specific.contains(p)))
            .collect();
        let mut i = 0;
        facts.refs.retain(|_| {
            i += 1;
            keep[i - 1]
        });
    }

    // ---- imports
    {
        for m in &import_matches {
            let mut path: Option<Node> = None;
            let mut is_mod = false;
            let mut imported = Vec::new();
            for &(cn, node) in m {
                match cn {
                    "import" => path = Some(node),
                    "import.mod" => {
                        path = Some(node);
                        is_mod = true;
                    }
                    "import.name" => imported.push(text(node, src).to_string()),
                    _ => {}
                }
            }
            let Some(p) = path else { continue };
            let mut spec_text = unquote(text(p, src));
            if is_mod {
                spec_text = format!("mod:{spec_text}");
            }
            if spec_text.is_empty() {
                continue;
            }
            // Patterns may match the same statement once per imported name: merge them.
            if let Some(last) = facts.imports.last_mut() {
                if last.path == spec_text && last.line == line(p) {
                    for n in imported {
                        if !last.names.contains(&n) {
                            last.names.push(n);
                        }
                    }
                    continue;
                }
            }
            facts.imports.push(Import {
                path: spec_text,
                names: imported,
                line: line(p),
            });
        }
    }

    // ---- entry points
    {
        let mut entry_names = Vec::new();
        for m in &entry_matches {
            for &(cn, node) in m {
                match cn {
                    "entry" => {
                        if let Some(&i) = def_at_name.get(&node.start_byte()) {
                            facts.defs[i].entry = true;
                        }
                    }
                    "entry.file" => facts.entry = true,
                    "entry.name" => entry_names.push(text(node, src).to_string()),
                    _ => {}
                }
            }
        }
        // Entry points referenced by name (route handlers): mark same-file definitions.
        for d in facts.defs.iter_mut() {
            if entry_names.contains(&d.name) {
                d.entry = true;
            }
        }
    }
    facts.namespaces.sort();
    facts.namespaces.dedup();
    let owners: Vec<Option<String>> = raw.iter().map(|d| d.owner.clone()).collect();
    let impls: Vec<bool> = raw.iter().map(|d| d.impl_block).collect();
    regroup(&mut facts, &owners, &impls);
    facts
}

/// Attaches methods to their type when the language declares them outside it: Go methods join
/// their receiver type, Rust `impl` blocks merge into the struct/enum/trait of the same name.
/// Defs are then re-ordered so every parent precedes its children.
fn regroup(facts: &mut FileFacts, owners: &[Option<String>], impls: &[bool]) {
    let n = facts.defs.len();
    if !impls.iter().any(|&b| b) && owners.iter().all(|o| o.is_none()) {
        return;
    }
    // First non-impl class/type definition of each name.
    let mut types: HashMap<String, usize> = HashMap::new();
    for (i, d) in facts.defs.iter().enumerate() {
        if !impls[i] && matches!(d.kind, NodeKind::Class | NodeKind::Type) {
            types.entry(d.name.clone()).or_insert(i);
        }
    }
    let mut parent: Vec<Option<usize>> = facts
        .defs
        .iter()
        .map(|d| d.parent.map(|p| p as usize))
        .collect();
    // Defs replaced by another (merged impl blocks) map to their replacement.
    let mut merged: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        if impls[i] {
            if let Some(&t) = types.get(facts.defs[i].name.as_str()) {
                merged[i] = Some(t);
            }
        }
    }
    for i in 0..n {
        if let Some(p) = parent[i] {
            if let Some(t) = merged[p] {
                parent[i] = Some(t);
            }
        }
        if let (Some(o), None) = (&owners[i], parent[i]) {
            if let Some(&t) = types.get(o.as_str()) {
                if t != i {
                    parent[i] = Some(t);
                    if facts.defs[i].kind == NodeKind::Function {
                        facts.defs[i].kind = NodeKind::Method;
                    }
                }
            }
        }
    }
    // Depth-first order by start line, parents first.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for i in 0..n {
        if merged[i].is_some() {
            continue;
        }
        match parent[i] {
            Some(p) => children[p].push(i),
            None => roots.push(i),
        }
    }
    let by_line = |v: &mut Vec<usize>, defs: &[crate::facts::Def]| {
        v.sort_by_key(|&i| (defs[i].start_line, i))
    };
    by_line(&mut roots, &facts.defs);
    for c in children.iter_mut() {
        by_line(c, &facts.defs);
    }
    let mut order = Vec::with_capacity(n);
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        stack.extend(children[i].iter().rev());
    }
    let mut new_index = vec![usize::MAX; n];
    for (new, &old) in order.iter().enumerate() {
        new_index[old] = new;
    }
    let remap = |old: usize| -> usize {
        let target = merged[old].unwrap_or(old);
        new_index[target]
    };
    let old_defs = std::mem::take(&mut facts.defs);
    for &old in &order {
        let mut d = old_defs[old].clone();
        d.parent = parent[old].map(|p| remap(p) as u32);
        // A merged impl block's entry flag carries over to the type.
        facts.defs.push(d);
    }
    for (old, d) in old_defs.iter().enumerate() {
        if merged[old].is_some() && d.entry {
            facts.defs[remap(old)].entry = true;
        }
    }
    for r in facts.refs.iter_mut() {
        r.from = r.from.map(|f| remap(f as usize) as u32);
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '<' || c == '>');
    s.to_string()
}

/// Whether a definition is visible outside its file/module (for dead-code candidates).
fn exported(lang: Lang, node: Node, name: &str, src: &[u8]) -> bool {
    let head = |n: Node| -> &str {
        let t = text(n, src);
        let cut = t.find(name).unwrap_or(t.len()).min(t.len());
        &t[..cut]
    };
    let word = |h: &str, w: &str| {
        h.split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|x| x == w)
    };
    match lang {
        Lang::Python | Lang::Lua => {
            !name.starts_with('_') || (name.starts_with("__") && name.ends_with("__"))
        }
        Lang::Go => name.chars().next().is_some_and(|c| c.is_uppercase()),
        Lang::Rust => {
            // pub items, and methods of trait impls (callable through the trait).
            if node
                .child(0)
                .is_some_and(|c| c.kind() == "visibility_modifier")
            {
                return true;
            }
            let mut p = node.parent();
            while let Some(n) = p {
                if n.kind() == "impl_item" {
                    return n.child_by_field_name("trait").is_some();
                }
                if n.kind() == "trait_item" {
                    return true;
                }
                p = n.parent();
            }
            false
        }
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            let mut p = Some(node);
            for _ in 0..3 {
                let Some(n) = p else { break };
                if n.kind() == "export_statement" {
                    return true;
                }
                p = n.parent();
            }
            // Class members are public API unless marked private.
            let h = head(node);
            matches!(
                node.kind(),
                "method_definition"
                    | "method_signature"
                    | "public_field_definition"
                    | "abstract_method_signature"
            ) && !word(h, "private")
                && !name.starts_with('#')
        }
        Lang::C | Lang::Cpp => !word(head(node), "static"),
        Lang::Java | Lang::CSharp | Lang::Kotlin | Lang::Swift | Lang::Php => {
            let h = head(node);
            !word(h, "private") && !word(h, "fileprivate")
        }
        _ => true,
    }
}
