//! Adversarial tests for the wiring, not the helper.
//!
//! `write_scope_extension`'s own tests prove the decision function. These prove
//! the thing that was actually missing: that the decision is reached at all,
//! from a real request, and — more importantly — that it fails CLOSED when the
//! wave context is absent. An empty claim list reads as "nobody owns anything",
//! which would grant every out-of-scope write and silently delete the ownership
//! check; that is the failure mode this file exists to catch.

use super::agent_adapter::{WorkflowV2AgentRequest, validate_write_ownership_for_tests};
use super::write_scope_extension::WaveClaim;
use crate::{
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2WriteMode,
};

fn changed(paths: &[&str]) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        files_changed: paths
            .iter()
            .map(|path| crate::WorkflowV2FileRecord::new(*path))
            .collect(),
        ..Default::default()
    }
}

fn request(
    item_id: &str,
    declared: &[&str],
    wave: Option<Vec<WaveClaim>>,
) -> WorkflowV2AgentRequest {
    let mut call = WorkflowV2HostCall {
        id: item_id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        // Worktree is the mode the write coordinator runs waves under, and the
        // only mode that stamps a claim list onto the call.
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: Default::default(),
    };
    // The claim list rides on the call exactly as the write coordinator stamps
    // it, so these exercise the real vehicle rather than a convenient field.
    if let Some(wave) = wave {
        call.options.extra.insert(
            "wave_claims".to_string(),
            serde_json::to_value(&wave).expect("claims serialise"),
        );
    }
    WorkflowV2AgentRequest {
        call,
        role: "implementer".to_string(),
        task: "do the work".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: declared.iter().map(|p| (*p).to_string()).collect(),
        target_ownership_scopes: Vec::new(),
    }
}

/// THE failure mode. No wave context must mean "stay strict", never "nothing is
/// claimed, allow everything".
#[test]
fn without_wave_context_an_undeclared_file_is_still_rejected() {
    let mut result = changed(&["src/declared.rs", "src/forgotten.rs"]);
    let err = validate_write_ownership_for_tests(
        &request("item-a", &["src/declared.rs"], None),
        &mut result,
    )
    .expect_err("no wave context must not relax ownership");
    assert!(err.to_string().contains("src/forgotten.rs"), "got: {err}");
}

/// An EMPTY wave list grants everything, because nothing claims anything.
///
/// The name says what it does rather than what I would prefer: this is the
/// permissive case, and it is REACHABLE ONLY through a plumbing bug, because a
/// wave with no assignments runs no branches and therefore reaches no adapter.
/// It is precisely why the vehicle is an `Option` and why a missing or
/// malformed list yields `None` rather than an empty vec — see
/// `without_wave_context_an_undeclared_file_is_still_rejected`, which is the
/// case that actually protects the gate.
#[test]
fn an_empty_wave_list_grants_everything_and_is_unreachable_in_practice() {
    let mut result = changed(&["src/forgotten.rs"]);
    validate_write_ownership_for_tests(
        &request("item-a", &["src/declared.rs"], Some(Vec::new())),
        &mut result,
    )
    .expect("an empty wave claims nothing, so nothing is contested");
}

/// A claim in a path form that cannot be compared must refuse to extend, not
/// grant. If the two sides ever speak different path languages, every owned
/// path reads as unclaimed — over-granting silently, which is worse than the
/// discard this whole change replaces.
#[test]
fn an_uncomparable_claim_refuses_to_extend_rather_than_granting() {
    let wave = vec![
        WaveClaim::new("item-a", ["src/declared.rs".to_string()]),
        // Absolute, with no repository_root on the request to resolve it.
        WaveClaim::new("item-b", ["/elsewhere/src/forgotten.rs".to_string()]),
    ];
    let mut result = changed(&["src/declared.rs", "src/forgotten.rs"]);
    let err = validate_write_ownership_for_tests(
        &request("item-a", &["src/declared.rs"], Some(wave)),
        &mut result,
    )
    .expect_err("an uncomparable claim must fail closed");
    assert!(err.to_string().contains("src/forgotten.rs"), "got: {err}");
}

/// The live failure this fixes: one unlisted path, correct work, discarded.
#[test]
fn a_file_no_other_item_claims_is_granted() {
    let wave = vec![
        WaveClaim::new("item-a", ["src/declared.rs".to_string()]),
        WaveClaim::new("item-b", ["src/other.rs".to_string()]),
    ];
    let mut result = changed(&["src/declared.rs", "src/forgotten.rs"]);
    validate_write_ownership_for_tests(
        &request("item-a", &["src/declared.rs"], Some(wave)),
        &mut result,
    )
    .expect("an unclaimed path must be granted, not discarded");
}

/// A genuine dispute still fails, and still names the path.
#[test]
fn a_file_another_item_owns_is_still_refused() {
    let wave = vec![
        WaveClaim::new("item-a", ["src/declared.rs".to_string()]),
        WaveClaim::new("item-b", ["src/contested.rs".to_string()]),
    ];
    let mut result = changed(&["src/declared.rs", "src/contested.rs"]);
    let err = validate_write_ownership_for_tests(
        &request("item-a", &["src/declared.rs"], Some(wave)),
        &mut result,
    )
    .expect_err("a path another item owns must not be granted");
    assert!(err.to_string().contains("src/contested.rs"), "got: {err}");
}

/// An item must not contest itself: its own claim is in the wave list.
#[test]
fn an_item_does_not_block_itself_via_its_own_claim() {
    let wave = vec![WaveClaim::new(
        "item-a",
        [
            "src/declared.rs".to_string(),
            "src/also-mine.rs".to_string(),
        ],
    )];
    let mut result = changed(&["src/also-mine.rs"]);
    validate_write_ownership_for_tests(
        &request("item-a", &["src/declared.rs"], Some(wave)),
        &mut result,
    )
    .expect("an item's own wave claim must not block it");
}
