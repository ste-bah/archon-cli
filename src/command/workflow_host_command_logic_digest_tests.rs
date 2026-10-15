use std::path::{Path, PathBuf};

use super::super::workflow_host_command_logic::{CAPABILITY_LOGIC, LOGIC_DENYLIST};
use super::super::workflow_host_command_logic_closure::Workspace;
use super::super::workflow_host_command_logic_source::{SourceTree, hashed_text_for_file};
use super::{sources_digest, sources_digest_from};

const REGISTRY: &str = "src/command/workflow_host_command_logic.rs";
const CATALOG: &str = "src/command/workflow_host_command_logic_catalog.rs";

/// Changes exactly one character inside a named function body. The assertions
/// deliberately name the expected mutations before the digest runs: pins are
/// digest outputs, while these two bodies are executable reuse logic.
fn change_function_character(source: &str, function: &str, from: &str, to: &str) -> String {
    assert_eq!(from.len(), 1);
    assert_eq!(to.len(), 1);
    let start = source
        .find(&format!("fn {function}"))
        .unwrap_or_else(|| panic!("real registry is missing fn {function}"));
    let open = source[start..]
        .find('{')
        .map(|at| start + at)
        .unwrap_or_else(|| panic!("fn {function} has no body"));
    let mut depth = 0usize;
    let close = source.as_bytes()[open..]
        .iter()
        .position(|byte| {
            match byte {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            depth == 0
        })
        .map(|at| open + at)
        .unwrap_or_else(|| panic!("fn {function} has an unterminated body"));
    let body = &source[open..=close];
    let at = body
        .find(from)
        .unwrap_or_else(|| panic!("fn {function} body does not contain {from:?}"));
    let mut changed = source.to_owned();
    changed.replace_range(open + at..open + at + 1, to);
    changed
}

fn dependent_capabilities(repo: &Path) -> Vec<&'static str> {
    let registry = Path::new(REGISTRY);
    let mut workspace = Workspace::new(repo);
    CAPABILITY_LOGIC
        .iter()
        .filter_map(|logic| {
            let roots = logic.sources.concat();
            let hashed = sources_digest(&mut workspace, repo, &roots, LOGIC_DENYLIST);
            hashed.closure.files.contains(registry).then_some(logic.id)
        })
        .collect()
}

fn capability_digests(repo: &Path, capabilities: &[&str], registry_text: String) -> Vec<String> {
    let catalog_text = SourceTree::checkout(repo).read_to_string(Path::new(CATALOG));
    capability_digests_with_catalog(repo, capabilities, registry_text, catalog_text)
}

fn capability_digests_with_catalog(
    repo: &Path,
    capabilities: &[&str],
    registry_text: String,
    catalog_text: String,
) -> Vec<String> {
    capabilities
        .iter()
        .map(|id| {
            let logic = CAPABILITY_LOGIC
                .iter()
                .find(|logic| logic.id == *id)
                .unwrap();
            let roots = logic.sources.concat();
            let mut workspace = Workspace::new(repo);
            let source = SourceTree::checkout(repo)
                .with_file(Path::new(REGISTRY), registry_text.clone())
                .with_file(Path::new(CATALOG), catalog_text.clone());
            let hashed = sources_digest_from(&mut workspace, &source, &roots, LOGIC_DENYLIST);
            assert!(hashed.closure.files.contains(Path::new(REGISTRY)), "{id}");
            hashed.digest
        })
        .collect()
}

#[test]
fn logic_361_registry_pins_are_normalized_and_reuse_code_is_hashed() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let real_registry = SourceTree::checkout(&repo).read_to_string(Path::new(REGISTRY));
    let capabilities = dependent_capabilities(&repo);
    assert!(
        !capabilities.is_empty(),
        "a capability must include {REGISTRY}"
    );
    for id in [
        "freeze-acceptance",
        "freeze-skeleton",
        "verify-frozen-acceptance",
        "verify-frozen-skeleton",
    ] {
        assert!(capabilities.contains(&id), "{id} must include {REGISTRY}");
    }

    // Expected: changing a real catalog pin leaves every digest equal; changing one
    // character in either real function body changes every dependent digest.
    let original = capability_digests(&repo, &capabilities, real_registry.clone());
    let real_catalog = SourceTree::checkout(&repo).read_to_string(Path::new(CATALOG));
    let pin_line = real_catalog
        .lines()
        .find(|line| line.trim_start().starts_with("sources_digest: \""))
        .expect("the real capability catalog must contain a pin line");
    let pin_at = real_catalog.find(pin_line).unwrap() + pin_line.find('"').unwrap() + 1;
    let mut changed_pin = real_catalog.clone();
    let changed_hex = if &changed_pin[pin_at..pin_at + 1] == "0" {
        "1"
    } else {
        "0"
    };
    changed_pin.replace_range(pin_at..pin_at + 1, changed_hex);
    assert_eq!(
        capability_digests_with_catalog(&repo, &capabilities, real_registry.clone(), changed_pin,),
        original,
        "changing one real sources_digest value preserves every dependent digest"
    );

    for (function, from, to) in [("key_token", "!", "="), ("outcome_logic_holds", "=", "!")] {
        let changed = change_function_character(&real_registry, function, from, to);
        let changed_digests = capability_digests(&repo, &capabilities, changed);
        for (id, (actual, expected)) in capabilities
            .iter()
            .zip(changed_digests.iter().zip(&original))
        {
            assert_ne!(actual, expected, "{id}: {function} must be hashed");
        }
    }
    assert_ne!(
        hashed_text_for_file(
            "src/command/other.rs",
            "sources_digest: \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\n",
        ),
        hashed_text_for_file(
            "src/command/other.rs",
            "sources_digest: \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\n",
        ),
        "the normalization is keyed only to the registry path"
    );
    assert_eq!(
        hashed_text_for_file(
            CATALOG,
            "sources_digest: \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\n",
        ),
        hashed_text_for_file(
            CATALOG,
            "sources_digest: \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\n",
        ),
        "catalog digest values are normalized as well"
    );
}

#[test]
fn logic_361_registry_is_hashed_by_every_dependent_capability() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut workspace = Workspace::new(&repo);
    let registry = Path::new("src/command/workflow_host_command_logic.rs");
    let dependents = [
        "freeze-acceptance",
        "freeze-skeleton",
        "verify-frozen-acceptance",
        "verify-frozen-skeleton",
    ];
    for id in dependents {
        let logic = CAPABILITY_LOGIC
            .iter()
            .find(|logic| logic.id == id)
            .unwrap();
        let roots = logic.sources.concat();
        let hashed = sources_digest(&mut workspace, &repo, &roots, &[]);
        assert!(
            hashed.closure.files.contains(registry),
            "{id} must hash the executable registry logic"
        );
    }
}
