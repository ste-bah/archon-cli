//! Issue 361: the logic walk's soundness check can fail, and the hashed
//! text never hides code.
use std::path::Path;

use crate::command::workflow_host_command_logic_closure::Closure;
use crate::command::workflow_host_command_logic_guard_tests::{Hashed, Tree, walk_problems};
use crate::command::workflow_host_command_logic_source::hashed_text;

/// A production module whose name reads as a test (here `*fixture*`) is
/// dropped from the closure; the soundness check must say so, never pass.
#[test]
fn logic_361_a_dropped_production_module_fails_the_soundness_check() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;\nmod rule_fixture;\n");
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::rule_fixture::judge() }\n",
    );
    tree.write(
        "src/command/rule_fixture.rs",
        "pub fn judge() -> bool { true }\n",
    );
    let hashed = tree.hash(&[]);
    assert!(
        !hashed
            .closure
            .files
            .contains(Path::new("src/command/rule_fixture.rs"))
    );
    let found = walk_problems(&hashed);
    assert!(
        found.iter().any(|p| p.contains("rule_fixture")),
        "the verdict reads an unhashed module: {found:?}"
    );
    // The same through a `use`.
    tree.write(
        "src/command/root.rs",
        "use super::rule_fixture::judge;\npub fn verdict() -> bool { judge() }\n",
    );
    let found = walk_problems(&tree.hash(&[]));
    assert!(
        found.iter().any(|p| p.contains("rule_fixture")),
        "{found:?}"
    );
}

/// A reference the walk recorded but then dropped (neither hashed nor
/// denied) is a problem, even with no unresolved reference.
#[test]
fn logic_361_a_reference_to_a_dropped_file_fails_the_soundness_check() {
    let mut closure = Closure::default();
    let (root, dropped) = (Path::new("src/root.rs"), Path::new("src/rule_fixture.rs"));
    closure.files.insert(root.to_path_buf());
    closure
        .references
        .entry(root.to_path_buf())
        .or_default()
        .insert(dropped.to_path_buf());
    let hashed = Hashed {
        digest: String::new(),
        closure,
        errors: Default::default(),
    };
    let found = walk_problems(&hashed);
    assert!(
        found
            .iter()
            .any(|p| p.contains("neither hashed nor denied")),
        "{found:?}"
    );
}

/// A reference into a denied file is recorded, so the denied leg of the
/// check sees it.
#[test]
fn logic_361_a_reference_into_the_denylist_is_recorded() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;\nmod transport;\n");
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::transport::send() }\n",
    );
    tree.write(
        "src/command/transport.rs",
        "pub fn send() -> bool { true }\n",
    );
    let hashed = tree.hash(&[("src/command/transport.rs", "transport only")]);
    let targets = &hashed.closure.references[Path::new("src/command/root.rs")];
    assert!(targets.contains(Path::new("src/command/transport.rs")));
    assert!(
        walk_problems(&hashed).is_empty(),
        "{:?}",
        walk_problems(&hashed)
    );
}

/// Code after the closing brace of a trailing test block is hashed.
#[test]
fn logic_361_code_on_the_closing_line_of_a_test_block_is_hashed() {
    let file = |value: u32| {
        format!(
            "pub fn a() {{}}\n#[cfg(test)]\nmod tests {{\n    #[test]\n    fn t() {{}}\n}} \
             pub fn real() -> u32 {{ {value} }}\n"
        )
    };
    assert_ne!(hashed_text(&file(1)), hashed_text(&file(2)));
    // A plain trailing test block is still cut.
    let plain = |body: &str| {
        format!("pub fn a() {{}}\n#[cfg(test)]\nmod tests {{\n    fn t() {{ {body} }}\n}}\n")
    };
    assert_eq!(hashed_text(&plain("")), hashed_text(&plain("1;")));
}

/// Literal names emitted by a declarative macro resolve to its definition;
/// changing either the definition or its invocation must invalidate the digest.
#[test]
fn logic_361_references_to_macro_generated_items_reach_the_definition_and_invocation() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;\nmod policy;\n");
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::policy::FIELDS.len() > 0 }\n",
    );
    let policy = "macro_rules! fields { ($($field:ident),+) => { pub const FIELDS: &[&str] = &[$(stringify!($field)),+]; }; }\nfields! { first, second }\n";
    tree.write("src/command/policy.rs", policy);
    let hashed = tree.hash(&[]);
    assert!(
        walk_problems(&hashed).is_empty(),
        "{:?}",
        walk_problems(&hashed)
    );
    assert!(tree.files().contains("src/command/policy.rs"));
    tree.write(
        "src/command/policy.rs",
        &policy.replace("first, second", "first, third"),
    );
    assert_ne!(tree.digest(), hashed.digest);
    let invocation_changed = tree.digest();
    tree.write(
        "src/command/policy.rs",
        &policy
            .replace("first, second", "first, third")
            .replace("stringify!($field)", "stringify!(other)"),
    );
    assert_ne!(tree.digest(), invocation_changed);
}

/// A macro's generated constant resolves at its definition, but the field
/// list lives in a different module which the forward walk need not visit.
#[test]
fn logic_361_cross_file_macro_arguments_cannot_escape_the_closure() {
    let tree = Tree::new();
    tree.write(
        "src/command/mod.rs",
        "mod root; mod policy; mod declaration;\n",
    );
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::policy::FIELDS.len() > 0 }\n",
    );
    tree.write("src/command/policy.rs", "macro_rules! fields { ($($field:ident),+) => { pub const FIELDS: &[&str] = &[$(stringify!($field)),+]; }; }\n");
    tree.write("src/command/declaration.rs", "fields! { first, second }\n");
    let hashed = tree.hash(&[]);
    assert!(
        !hashed
            .closure
            .files
            .contains(Path::new("src/command/declaration.rs"))
    );
    let problems = walk_problems(&hashed);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("declaration.rs") && p.contains("macro invocation fields!")),
        "{problems:?}"
    );
    tree.write(
        "src/command/declaration.rs",
        "use super::policy::fields as selected; selected! { first, second }\n",
    );
    let aliased = walk_problems(&tree.hash(&[]));
    assert!(
        aliased
            .iter()
            .any(|p| p.contains("macro invocation selected!")),
        "{aliased:?}"
    );
    tree.write("src/command/declaration.rs", "fields! { first, second }\n");
    // Explicitly reaching the invocation closes the hole and hashes its fields.
    tree.write("src/command/mod.rs", "mod root; mod policy;\n");
    tree.write("src/command/root.rs", "pub fn verdict() -> bool { super::policy::FIELDS.len() > 0 }\n#[path = \"declaration.rs\"] mod declaration;\n");
    let reached = tree.hash(&[]);
    assert!(
        walk_problems(&reached).is_empty(),
        "{:?}",
        walk_problems(&reached)
    );
    assert!(
        reached
            .closure
            .files
            .contains(Path::new("src/command/declaration.rs"))
    );
    tree.write("src/command/declaration.rs", "fields! { first, third }\n");
    assert_ne!(tree.digest(), reached.digest);
}

/// A same-named macro with its own resolved definition is independent.
#[test]
fn logic_361_an_unrelated_local_macro_does_not_expand_the_closure() {
    let tree = Tree::new();
    tree.write(
        "src/command/mod.rs",
        "mod root; mod policy; mod separate;\n",
    );
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::policy::FIELDS.len() > 0 }\n",
    );
    tree.write(
        "src/command/policy.rs",
        "macro_rules! fields { () => { pub const FIELDS: &[&str] = &[]; }; } fields!();\n",
    );
    tree.write(
        "src/command/separate.rs",
        "macro_rules! fields { () => { pub const OTHER: bool = true; }; } fields!();\n",
    );
    let hashed = tree.hash(&[]);
    assert!(
        !hashed
            .closure
            .files
            .contains(Path::new("src/command/separate.rs"))
    );
    assert!(
        walk_problems(&hashed).is_empty(),
        "{:?}",
        walk_problems(&hashed)
    );
}

#[test]
fn logic_361_follows_reexported_modules_across_crate_boundaries() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;");
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { rules_crate::bridge::decision() }",
    );
    tree.write(
        "crates/rules/src/lib.rs",
        "pub use sibling::inner as bridge;",
    );
    tree.write(
        "crates/sibling/Cargo.toml",
        "[package]\nname = \"sibling\"\n",
    );
    tree.write("crates/sibling/src/lib.rs", "pub mod inner;");
    tree.write(
        "crates/sibling/src/inner.rs",
        "pub fn decision() -> bool { true }",
    );
    let hashed = tree.hash(&[]);
    assert!(
        walk_problems(&hashed).is_empty(),
        "{:?}",
        walk_problems(&hashed)
    );
    assert!(
        hashed
            .closure
            .files
            .contains(Path::new("crates/sibling/src/inner.rs"))
    );
}
