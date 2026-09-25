//! What a function node is to the scan: a declaration, a callback, or a
//! container.
//!
//! - A **declaration** (and a function value bound to a name) absorbs every
//!   function nested in it.
//! - A **callback** — any other anonymous function — absorbs the callbacks
//!   nested in it, so logic cannot be spread over nested closures to stay
//!   under the cap. The declarations and containers inside it are judged on
//!   their own.
//! - A **container** scores only its own tokens; everything inside it is
//!   judged on its own. Only genuine grouping constructs are containers, from
//!   a small generic allowlist: test-structure callbacks (`describe`,
//!   `context`, `it`, `test`, `suite`, `specify`, the `before*` / `after*`
//!   hooks, with `.each` / `.only` / `.skip` / `.todo` / `.concurrent` /
//!   `.failing`; Ruby `describe` / `RSpec.describe` / `context` / `it` /
//!   `specify` / `let` / `before` / `after` blocks) and a file-root IIFE or
//!   UMD wrapper. Without them a test suite or module wrapper added every
//!   case to one score.

use tree_sitter::Node;

use super::tree_names::{call_callee, is_declaration};
use super::tree_scan::Grammar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Declaration,
    Callback,
    Container,
}

const TEST_CALLEES: &[&str] = &[
    "describe",
    "context",
    "it",
    "test",
    "suite",
    "specify",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
];

const TEST_MODIFIERS: &[&str] = &["each", "only", "skip", "todo", "concurrent", "failing"];

const RUBY_BLOCK_CALLEES: &[&str] = &[
    "describe", "context", "it", "specify", "let", "let!", "before", "after",
];

pub(super) fn role(grammar: Grammar, node: Node, text: &str) -> Role {
    if is_declaration(node) {
        Role::Declaration
    } else if is_container(grammar, node, text) {
        Role::Container
    } else {
        Role::Callback
    }
}

fn is_container(grammar: Grammar, node: Node, text: &str) -> bool {
    match grammar {
        Grammar::TypeScript | Grammar::Tsx => test_callback(node, text) || root_wrapper(node),
        Grammar::Ruby => ruby_test_block(node, text),
        _ => false,
    }
}

/// A callback passed to `describe(...)`, `it.each(...)(...)` and the like.
fn test_callback(node: Node, text: &str) -> bool {
    let Some(call) = node
        .parent()
        .filter(|parent| parent.kind() == "arguments")
        .and_then(|arguments| arguments.parent())
    else {
        return false;
    };
    let callee = call_callee(call, text).unwrap_or_default();
    let mut parts = callee.split('.');
    parts
        .next()
        .is_some_and(|first| TEST_CALLEES.contains(&first))
        && parts.all(|part| TEST_MODIFIERS.contains(&part))
}

/// The function of a file-root IIFE — `(function () { ... })()`, `!function
/// () {}()` — or the factory passed to one (UMD).
fn root_wrapper(node: Node) -> bool {
    let mut current = node;
    let mut parent = node.parent();
    while let Some(up) =
        parent.filter(|up| matches!(up.kind(), "parenthesized_expression" | "unary_expression"))
    {
        current = up;
        parent = up.parent();
    }
    let Some(call) = parent else {
        return false;
    };
    let invoked = call.kind() == "call_expression"
        && call
            .child_by_field_name("function")
            .is_some_and(|function| function.id() == current.id());
    let factory = call.kind() == "arguments"
        && call
            .parent()
            .is_some_and(|outer| outer.kind() == "call_expression" && invokes_function(outer));
    let call = if factory { call.parent() } else { Some(call) };
    (invoked || factory) && call.is_some_and(at_file_root)
}

/// Whether a call's callee is a function expression (an IIFE).
fn invokes_function(call: Node) -> bool {
    let mut callee = call.child_by_field_name("function");
    while let Some(inner) = callee.filter(|inner| inner.kind() == "parenthesized_expression") {
        callee = inner.named_child(0);
    }
    callee.is_some_and(|callee| {
        matches!(
            callee.kind(),
            "function_expression" | "function" | "arrow_function"
        )
    })
}

/// Whether an expression is a statement directly in the file (possibly
/// wrapped in `!`, `void` or parentheses).
fn at_file_root(expression: Node) -> bool {
    let mut current = expression.parent();
    while let Some(up) =
        current.filter(|up| matches!(up.kind(), "parenthesized_expression" | "unary_expression"))
    {
        current = up.parent();
    }
    current.is_some_and(|statement| {
        statement.kind() == "expression_statement"
            && statement
                .parent()
                .is_some_and(|root| root.kind() == "program")
    })
}

/// A Ruby block attached to `describe` / `RSpec.describe` / `it` / `let` ...
fn ruby_test_block(node: Node, text: &str) -> bool {
    let Some(call) = node
        .parent()
        .filter(|call| call.kind() == "call" && call.child_by_field_name("block") == Some(node))
    else {
        return false;
    };
    let method = call
        .child_by_field_name("method")
        .map(|method| &text[method.byte_range()]);
    let receiver = call
        .child_by_field_name("receiver")
        .map(|receiver| &text[receiver.byte_range()]);
    method.is_some_and(|method| RUBY_BLOCK_CALLEES.contains(&method))
        && matches!(receiver, None | Some("RSpec"))
}
