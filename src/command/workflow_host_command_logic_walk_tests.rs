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
