//! `SharedAppend` — the three overlap rules, and the declaration that builds it.
//!
//! The property under test is that concurrency is opted into. Every path is
//! exclusive until something names it, one side declaring shared is not enough,
//! and a claim that cannot be resolved conflicts rather than passing.

use super::*;
use crate::write_plan::{keys_conflict, normalize_target};
use serde_json::json;

fn root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn shared(path: &str) -> ResourceKey {
    shared_append_key_for_raw_target(path)
}

fn file(path: &str) -> ResourceKey {
    ResourceKey::File(fold_resource_case(path))
}

fn dir(path: &str) -> ResourceKey {
    ResourceKey::Dir(fold_resource_case(path))
}

fn glob(pattern: &str) -> ResourceKey {
    ResourceKey::Glob(fold_resource_case(pattern))
}

// ------------------------------------------------------------- overlap rules

#[test]
fn two_shared_appends_on_one_path_do_not_conflict() {
    let a = shared(".archon/data/registry.json");
    let b = shared(".archon/data/registry.json");
    assert!(
        !keys_conflict(&a, &b),
        "both parties assert coordinated access"
    );
    assert!(!keys_conflict(&a, &shared(".archon/data/other.json")));
}

#[test]
fn a_shared_append_conflicts_with_any_exclusive_claim_on_the_same_path() {
    let appended = shared(".archon/data/registry.json");
    assert!(
        keys_conflict(&appended, &file(".archon/data/registry.json")),
        "one side wants the file to itself"
    );
    assert!(
        keys_conflict(&file(".archon/data/registry.json"), &appended),
        "and the table is symmetric"
    );
    assert!(!keys_conflict(&appended, &file(".archon/data/other.json")));
}

#[test]
fn a_shared_append_conflicts_with_a_dir_or_glob_covering_it() {
    let appended = shared(".archon/data/registry.json");
    assert!(keys_conflict(&appended, &dir(".archon/data")));
    assert!(keys_conflict(&appended, &dir(".archon")));
    assert!(!keys_conflict(&appended, &dir(".archon/other")));
    assert!(
        !keys_conflict(&appended, &dir(".archon/dat")),
        "no prefix false-positive"
    );
    assert!(keys_conflict(&appended, &glob(".archon/data/*.json")));
    assert!(!keys_conflict(&appended, &glob("src/*.rs")));
}

/// The fail-safe that must not weaken. `write_plan`'s `glob_match` already
/// treats a malformed pattern as conflicting; a shared-append claim that names
/// a pattern, or nothing at all, is the same species of unreadable claim.
#[test]
fn an_unresolvable_shared_append_conflicts_with_everything() {
    for raw in ["", "   ", ".archon/data/*.json", "logs/[bad"] {
        let unresolvable = shared(raw);
        assert!(
            keys_conflict(&unresolvable, &shared(".archon/data/registry.json")),
            "'{raw}' must not pass as a coordinated claim"
        );
        assert!(
            keys_conflict(&unresolvable, &file("src/lib.rs")),
            "'{raw}' vs an unrelated file"
        );
        assert!(
            keys_conflict(&file("src/lib.rs"), &unresolvable),
            "'{raw}' symmetric"
        );
    }
}

#[test]
fn a_malformed_glob_still_conflicts_with_a_shared_append() {
    assert!(keys_conflict(
        &glob("src/[bad"),
        &shared(".archon/data/registry.json")
    ));
}

#[test]
fn shared_append_sorts_after_the_three_exclusive_kinds() {
    let set: std::collections::BTreeSet<ResourceKey> = [
        ResourceKey::SharedAppend("a".into()),
        ResourceKey::Glob("a".into()),
        ResourceKey::Dir("a".into()),
        ResourceKey::File("a".into()),
    ]
    .into_iter()
    .collect();
    let order: Vec<ResourceKey> = set.into_iter().collect();
    assert_eq!(
        order,
        vec![
            ResourceKey::File("a".into()),
            ResourceKey::Dir("a".into()),
            ResourceKey::Glob("a".into()),
            ResourceKey::SharedAppend("a".into()),
        ]
    );
}

// --------------------------------------------------------- the declaration

#[test]
fn an_item_declaring_nothing_shares_nothing() {
    for payload in [
        json!({"target_files": ["a.rs"]}),
        json!({"target_files": ["a.rs"], "shared_append_target_files": []}),
        json!({"target_files": ["a.rs"], "shared_append_target_files": null}),
    ] {
        assert!(
            resolve_shared_append_targets(&payload)
                .expect("absent or empty is not an error")
                .is_empty(),
            "{payload}"
        );
    }
}

#[test]
fn a_malformed_shared_append_declaration_errs_rather_than_shrinking() {
    for payload in [
        json!({"shared_append_target_files": "registry.json"}),
        json!({"shared_append_target_files": [42]}),
        json!({"shared_append_target_files": ["ok.json", true]}),
    ] {
        assert!(
            resolve_shared_append_targets(&payload).is_err(),
            "a broken concurrency declaration must not read as a smaller one: {payload}"
        );
    }
}

#[test]
fn declared_paths_come_back_in_order() {
    let payload = json!({"shared_append_target_files": ["b.json", "a.json"]});
    assert_eq!(
        resolve_shared_append_targets(&payload).expect("ok"),
        vec!["b.json".to_string(), "a.json".to_string()]
    );
}

// ------------------------------------------------------------- key building

#[test]
fn a_shared_target_replaces_its_file_key_and_creates_no_dir_keys() {
    let r = root();
    let registry = normalize_target(".archon/data/registry.json", r.path()).expect("normalize");
    let manifest = normalize_target("out/mine/manifest.json", r.path()).expect("normalize");
    let keys = resource_keys_for_targets_with_shared_append(
        &[registry.clone(), manifest],
        r.path(),
        &[],
        &[registry],
    )
    .expect("keys");

    assert!(keys.contains(&shared(".archon/data/registry.json")));
    assert!(
        !keys.contains(&file(".archon/data/registry.json")),
        "a File key beside the SharedAppend key would defeat the declaration"
    );
    assert!(
        !keys.contains(&dir(".archon/data")),
        "you cannot coordinate an append while racing to create the directory"
    );
    assert!(
        keys.contains(&file("out/mine/manifest.json")),
        "the exclusive target is untouched"
    );
    assert!(
        keys.contains(&dir("out/mine")),
        "and still claims the directory it creates"
    );
}

#[test]
fn declaring_nothing_shared_reproduces_the_exclusive_key_set() {
    let r = root();
    let target = normalize_target("out/a.json", r.path()).expect("normalize");
    let with_none = resource_keys_for_targets_with_shared_append(
        std::slice::from_ref(&target),
        r.path(),
        &[],
        &[],
    )
    .expect("keys");
    let plain =
        crate::write_plan::resource_keys_for_targets(std::slice::from_ref(&target), r.path(), &[])
            .expect("keys");
    assert_eq!(with_none, plain);
}

/// The four-way case this key exists for: several items appending to one
/// registry, each also writing files it owns alone.
#[test]
fn four_items_appending_to_one_registry_do_not_conflict_pairwise() {
    let r = root();
    std::fs::create_dir_all(r.path().join(".archon/data")).expect("registry dir exists");
    let registry = normalize_target(".archon/data/registry.json", r.path()).expect("normalize");

    let key_sets: Vec<std::collections::BTreeSet<ResourceKey>> = ["w", "x", "y", "z"]
        .iter()
        .map(|owner| {
            std::fs::create_dir_all(r.path().join("out").join(owner)).expect("own dir exists");
            let own = normalize_target(&format!("out/{owner}/manifest.json"), r.path())
                .expect("normalize");
            resource_keys_for_targets_with_shared_append(
                &[registry.clone(), own],
                r.path(),
                &[],
                std::slice::from_ref(&registry),
            )
            .expect("keys")
        })
        .collect();

    for (i, left) in key_sets.iter().enumerate() {
        for right in &key_sets[i + 1..] {
            assert!(
                !left
                    .iter()
                    .any(|a| right.iter().any(|b| keys_conflict(a, b))),
                "items {i} and later must plan concurrently"
            );
        }
    }
}
