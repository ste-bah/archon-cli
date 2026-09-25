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
//!   judged on its own. It is recognised by structure, in any language: a
//!   callback that holds two or more callbacks as statement-level call
//!   arguments or blocks (a test suite, a route table, a Rake namespace,
//!   `$(function () { ... on(...) ... on(...) })`), or a file-root IIFE / UMD
//!   wrapper (`(function () {})()`, `(function () {}).call(this)`). Test
//!   callbacks are also containers by name (`describe`, `it`, `test`, the
//!   hooks and their `x`/`f`/`.each`/`.only`/`.skip` forms; Ruby
//!   `describe` / `it` / `let` ... blocks), so a suite holding one case does
//!   not change shape when a second is added. Without containers a suite or
//!   wrapper added every case to one score.

use tree_sitter::Node;

use super::tree_names::{call_callee, is_declaration, is_function};
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
    "xdescribe",
    "xcontext",
    "xit",
    "xtest",
    "fdescribe",
    "fit",
];

const TEST_MODIFIERS: &[&str] = &[
    "each",
    "only",
    "skip",
    "todo",
    "concurrent",
    "failing",
    "skipIf",
    "runIf",
    "describe",
    "serial",
    "parallel",
];

const RUBY_BLOCK_CALLEES: &[&str] = &[
    "describe",
    "context",
    "it",
    "specify",
    "let",
    "let!",
    "before",
    "after",
    "feature",
    "scenario",
    "background",
    "namespace",
    "task",
];

/// Ginkgo's blocks, and `t.Run` subtests.
const GO_CALLEES: &[&str] = &[
    "Describe",
    "Context",
    "When",
    "It",
    "BeforeEach",
    "AfterEach",
    "JustBeforeEach",
    "t.Run",
];

/// `node:test` / tap subtests: `t.test(...)`, `t.run(...)`.
const SUBTEST_CALLEES: &[&str] = &["t.test", "t.run", "t.describe"];

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
    let named = match grammar {
        Grammar::TypeScript | Grammar::Tsx => test_callback(node, text) || root_wrapper(node),
        Grammar::Ruby => ruby_test_block(node, text),
        Grammar::Go => {
            passed_to(node, text).is_some_and(|callee| GO_CALLEES.contains(&callee.as_str()))
        }
        _ => false,
    };
    named || held_callbacks(grammar, node) >= 2
}

/// Callbacks `node` holds as arguments (or a Ruby block) of calls that are
/// statements directly in its body.
fn held_callbacks(grammar: Grammar, node: Node) -> usize {
    let Some(body) = node.child_by_field_name("body") else {
        return 0;
    };
    statements(body)
        .into_iter()
        .filter_map(statement_call)
        .map(|call| callbacks_of(grammar, call))
        .sum()
}

/// A body's statements, looking through a statement-list wrapper.
fn statements(body: Node) -> Vec<Node> {
    let mut cursor = body.walk();
    let mut out = Vec::new();
    for child in body.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "statement_list" | "block_body" | "body_statement"
        ) {
            let mut inner = child.walk();
            out.extend(child.named_children(&mut inner));
        } else {
            out.push(child);
        }
    }
    out
}

/// The call a statement is, looking through `expression_statement` and
/// `await`.
fn statement_call(statement: Node) -> Option<Node> {
    let mut current = statement;
    for _ in 0..3 {
        if matches!(
            current.kind(),
            "call_expression" | "call" | "method_invocation"
        ) {
            return Some(current);
        }
        if !matches!(current.kind(), "expression_statement" | "await_expression") {
            return None;
        }
        current = current.named_child(0)?;
    }
    None
}

fn callbacks_of(grammar: Grammar, call: Node) -> usize {
    let is_callback = |node: Node| is_function(grammar, node) && !is_declaration(node);
    let arguments = call
        .child_by_field_name("arguments")
        .map_or(0, |arguments| {
            let mut cursor = arguments.walk();
            arguments
                .named_children(&mut cursor)
                .filter(|argument| is_callback(*argument))
                .count()
        });
    let block = call.child_by_field_name("block").is_some_and(is_callback);
    arguments + usize::from(block)
}

/// What a callback is passed to, by identifiers (`It`, `t.Run`).
fn passed_to(node: Node, text: &str) -> Option<String> {
    let arguments = node
        .parent()
        .filter(|parent| matches!(parent.kind(), "arguments" | "argument_list"))?;
    call_callee(arguments.parent()?, text)
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
    if SUBTEST_CALLEES.contains(&callee.as_str()) {
        return true;
    }
    let mut parts = callee.split('.');
    parts
        .next()
        .is_some_and(|first| TEST_CALLEES.contains(&first))
        && parts.all(|part| TEST_MODIFIERS.contains(&part))
}

/// The function of a file-root IIFE — `(function () { ... })()`, `!function
/// () {}()`, `(function () {}).call(this)` — or the factory passed to one
/// (UMD).
fn root_wrapper(node: Node) -> bool {
    let mut current = node;
    let mut parent = node.parent();
    while let Some(up) =
        parent.filter(|up| matches!(up.kind(), "parenthesized_expression" | "unary_expression"))
    {
        current = up;
        parent = up.parent();
    }
    // `(function () {}).call(this)` / `.apply(...)`: invoked through a member.
    if let Some(member) = parent.filter(|up| up.kind() == "member_expression")
        && member
            .child_by_field_name("object")
            .is_some_and(|object| object.id() == current.id())
    {
        current = member;
        parent = member.parent();
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
