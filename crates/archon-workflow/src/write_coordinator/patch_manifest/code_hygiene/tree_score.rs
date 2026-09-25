//! Scoring one function read from a syntax tree.
//!
//! A score is 1 plus one per branch keyword token and per `&&` / `||`
//! operator among the function's syntax tokens (its unnamed leaves), so
//! string, comment and docstring contents — named leaves — are never read.
//! A declared function scores every token under it, closures and nested
//! functions included. A container (an anonymous callback, IIFE or block
//! outside every declared function) scores only its own tokens: each
//! function-like node inside it is judged as a function of its own.

use tree_sitter::Node;

use super::tree_names::is_function;
use super::tree_scan::Grammar;
use super::{BRANCH_TOKENS, LOGICAL_OPERATORS, RUBY_BRANCH_TOKENS};

/// Branch tokens under `node`, skipping nested functions when `own_only`.
pub(super) fn branch_count(grammar: Grammar, node: Node, text: &str, own_only: bool) -> u32 {
    let extra: &[&str] = if grammar == Grammar::Ruby {
        RUBY_BRANCH_TOKENS
    } else {
        &[]
    };
    let mut count = 0usize;
    for leaf in leaves(grammar, node, own_only) {
        if leaf.is_named() || leaf.is_missing() {
            continue;
        }
        let kind = leaf.kind();
        // Ruby's `case` opens a `switch`; its `when` clauses are the branches.
        let counted = (BRANCH_TOKENS.contains(&kind)
            && !(grammar == Grammar::Ruby && kind == "case"))
            || extra.contains(&kind);
        if counted {
            count += 1;
        }
        let token = &text[leaf.byte_range()];
        count += LOGICAL_OPERATORS
            .iter()
            .map(|operator| token.matches(operator).count())
            .sum::<usize>();
    }
    count as u32
}

/// The text of `node`'s tokens before byte `end` (its body), comments left
/// out, literal contents kept.
pub(super) fn signature(grammar: Grammar, node: Node, end: usize, text: &str) -> String {
    leaves(grammar, node, false)
        .into_iter()
        .filter(|leaf| leaf.end_byte() <= end)
        .map(|leaf| &text[leaf.byte_range()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every leaf under `node` outside comments, in source order; with
/// `own_only`, also outside every function nested in it.
fn leaves(grammar: Grammar, node: Node, own_only: bool) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind().contains("comment") {
            continue;
        }
        if own_only && current.id() != node.id() && is_function(grammar, current) {
            continue;
        }
        if current.child_count() == 0 {
            out.push(current);
            continue;
        }
        let mut cursor = current.walk();
        let children: Vec<Node> = current.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}
