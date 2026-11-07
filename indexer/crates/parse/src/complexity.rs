//! Cyclomatic complexity per function, computed on demand (only for files someone looks at).
//! 1 + the number of decision points (branches, loops, case arms, catch clauses, ternaries and
//! short-circuit operators) inside the function, not counting nested functions.

use crate::extract::parse;
use crate::langs::spec;
use compass_core::model::Lang;
use tree_sitter::Node;

/// Node kinds that add a path through the code, across the Tier 1 grammars.
const DECISIONS: &[&str] = &[
    "if_statement",
    "if_expression",
    "elif_clause",
    "else_if_clause",
    "for_statement",
    "for_in_statement",
    "for_expression",
    "for_range_loop",
    "enhanced_for_statement",
    "foreach_statement",
    "while_statement",
    "while_expression",
    "do_statement",
    "loop_expression",
    "case_clause",
    "switch_case",
    "case_statement",
    "switch_section",
    "switch_label",
    "expression_case",
    "type_case",
    "communication_case",
    "match_arm",
    "when_entry",
    "catch_clause",
    "except_clause",
    "rescue",
    "conditional_expression",
    "ternary_expression",
    "conditional",
    "guard_statement",
    "&&",
    "||",
    "and",
    "or",
    "??",
];

/// Node kinds that open a new function scope (their decisions belong to them).
fn is_function(kind: &str) -> bool {
    kind.contains("function")
        || kind.contains("method")
        || kind == "lambda"
        || kind == "arrow_function"
        || kind == "closure_expression"
        || kind == "constructor_declaration"
        || kind == "init_declaration"
}

/// Complexity per function, keyed by the function's 1-based start line.
pub fn complexity(lang: Lang, src: &[u8]) -> Vec<(u32, u32)> {
    if spec(lang).is_none() {
        return Vec::new();
    }
    let Some(tree) = parse(lang, src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(tree.root_node(), &mut Vec::new(), &mut out);
    out.sort_unstable();
    out
}

/// Depth-first walk with a stack of open functions: (start line, decisions so far).
fn walk(node: Node, stack: &mut Vec<(u32, u32)>, out: &mut Vec<(u32, u32)>) {
    let opens = node.is_named() && is_function(node.kind());
    if opens {
        stack.push((node.start_position().row as u32 + 1, 0));
    } else if DECISIONS.contains(&node.kind()) {
        if let Some(top) = stack.last_mut() {
            top.1 += 1;
        }
    }
    let mut c = node.walk();
    for child in node.children(&mut c) {
        walk(child, stack, out);
    }
    if opens {
        let (line, decisions) = stack.pop().unwrap();
        out.push((line, decisions + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_branches_loops_and_operators() {
        let src = b"def f(x):\n    if x and x > 1:\n        for i in range(x):\n            pass\n    elif x:\n        return 1\n    return 0\n\ndef g():\n    return 1\n";
        assert_eq!(complexity(Lang::Python, src), vec![(1, 5), (9, 1)]);
    }

    #[test]
    fn nested_functions_keep_their_own_count() {
        let src = b"function outer(a) {\n  const inner = (b) => b ? 1 : 2;\n  if (a) { return inner(a); }\n  return 0;\n}\n";
        assert_eq!(complexity(Lang::TypeScript, src), vec![(1, 2), (2, 2)]);
    }
}
