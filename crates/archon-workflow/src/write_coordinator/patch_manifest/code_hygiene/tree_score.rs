//! Scoring one function read from a syntax tree.
//!
//! A score is 1 plus one per branch keyword token and per `&&` / `||`
//! operator among the function's syntax tokens (its unnamed leaves), so
//! string, comment and docstring contents — named leaves — are never read.
//! What a function leaves out of its score — the functions judged on their
//! own — follows its role (`tree_roles`).

use tree_sitter::Node;

use super::tree_scan::Grammar;
use super::{BRANCH_TOKENS, LOGICAL_OPERATORS, RUBY_BRANCH_TOKENS};

/// Branch tokens under `node`, leaving out the nested nodes `apart` picks
/// (functions judged on their own).
pub(super) fn branch_count(
    grammar: Grammar,
    node: Node,
    text: &str,
    apart: &dyn Fn(Node) -> bool,
) -> u32 {
    let extra: &[&str] = if grammar == Grammar::Ruby {
        RUBY_BRANCH_TOKENS
    } else {
        &[]
    };
    let ruby = grammar == Grammar::Ruby;
    let mut count = 0usize;
    for leaf in leaves(node, apart) {
        if leaf.is_named() || leaf.is_missing() {
            continue;
        }
        let kind = leaf.kind();
        // Ruby's `case` opens a `switch`; its `when` and pattern `in` clauses
        // are the branches.
        let pattern_arm = ruby
            && kind == "in"
            && leaf
                .parent()
                .is_some_and(|parent| parent.kind() == "in_clause");
        let counted = (BRANCH_TOKENS.contains(&kind) && !(ruby && kind == "case"))
            || extra.contains(&kind)
            || pattern_arm;
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
pub(super) fn signature(node: Node, end: usize, text: &str) -> String {
    leaves(node, &|_| false)
        .into_iter()
        .filter(|leaf| leaf.end_byte() <= end)
        .map(|leaf| &text[leaf.byte_range()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every leaf under `node` outside comments and outside the nested nodes
/// `apart` picks, in source order.
fn leaves<'tree>(node: Node<'tree>, apart: &dyn Fn(Node) -> bool) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind().contains("comment") {
            continue;
        }
        if current.id() != node.id() && apart(current) {
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
