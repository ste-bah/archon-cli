//! Tests for the gate that actually rejected the work.
//!
//! The adapter-level tests for this feature passed while the feature did
//! nothing, because a write branch clears THREE ownership gates and they only
//! exercised the first. These target the coordinator's plan — the one that
//! raises `UndeclaredWrite` — and the safety property that a granted path must
//! become a DECLARED path, or it is invisible to the overlap and stale-baseline
//! guards downstream.

use archon_write_plan::{TargetFilesSource, WritePlan, normalize_target};

use super::worktree_scope_grant::plan_extended_to_unclaimed_changes;
use crate::v2::write_scope_extension::WaveClaim;
use crate::{WorkflowV2FileRecord, WorkflowV2Result, WorkflowV2Status};

fn plan(item_id: &str, targets: &[&str]) -> WritePlan {
    let canonical_root = std::path::PathBuf::from("/repo");
    WritePlan {
        run_id: "wf-test".into(),
        stage_id: "implementation-wave-1".into(),
        item_id: item_id.into(),
        target_files: targets
            .iter()
            .map(|path| normalize_target(path, &canonical_root).expect("normalize"))
            .collect(),
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "base".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
        isolated_root: std::path::PathBuf::from("/tmp/iso"),
        canonical_root,
    }
}

fn changed(paths: &[&str]) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        files_changed: paths
            .iter()
            .map(|p| WorkflowV2FileRecord::new(*p))
            .collect(),
        ..Default::default()
    }
}

fn declared(plan: &WritePlan) -> Vec<String> {
    plan.target_files
        .iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

/// THE failure mode: no wave context must never widen the plan.
#[test]
fn without_wave_context_the_plan_is_untouched() {
    let base = plan("item-a", &["src/declared.rs"]);
    let extended = plan_extended_to_unclaimed_changes(&base, &changed(&["src/forgotten.rs"]), None);
    assert_eq!(declared(&extended), declared(&base));
}

/// The live failure: correct work, one unlisted path, previously discarded.
/// The granted path must be DECLARED, not merely tolerated — that is what puts
/// it under the overlap and stale-baseline guards at apply time.
#[test]
fn an_unclaimed_changed_file_becomes_a_declared_target() {
    let base = plan("item-a", &["src/declared.rs"]);
    let wave = vec![
        WaveClaim::new("item-a", ["src/declared.rs".to_string()]),
        WaveClaim::new("item-b", ["src/other.rs".to_string()]),
    ];
    let extended = plan_extended_to_unclaimed_changes(
        &base,
        &changed(&["src/declared.rs", "src/forgotten.rs"]),
        Some(&wave),
    );
    assert!(
        declared(&extended).contains(&"src/forgotten.rs".to_string()),
        "granted path must be declared so apply-time guards cover it: {:?}",
        declared(&extended)
    );
}

/// A genuine dispute is not granted, so the coordinator still refuses it.
#[test]
fn a_contested_file_is_not_granted() {
    let base = plan("item-a", &["src/declared.rs"]);
    let wave = vec![
        WaveClaim::new("item-a", ["src/declared.rs".to_string()]),
        WaveClaim::new("item-b", ["src/contested.rs".to_string()]),
    ];
    let extended =
        plan_extended_to_unclaimed_changes(&base, &changed(&["src/contested.rs"]), Some(&wave));
    assert_eq!(declared(&extended), declared(&base));
}

/// An empty wave list widens nothing here, because there is nothing changed
/// outside the plan to widen TO in the common case — but when there is, it
/// grants. Pinned so the permissive shape is visible rather than assumed.
#[test]
fn an_empty_wave_list_grants_an_unclaimed_path() {
    let base = plan("item-a", &["src/declared.rs"]);
    let extended =
        plan_extended_to_unclaimed_changes(&base, &changed(&["src/forgotten.rs"]), Some(&[]));
    assert!(declared(&extended).contains(&"src/forgotten.rs".to_string()));
}

/// A path already covered by a directory scope is not "outside", so it must not
/// be re-added as a redundant declared file.
#[test]
fn a_path_inside_a_declared_scope_is_not_re_granted() {
    let mut base = plan("item-a", &["src/declared.rs"]);
    base.target_dir_scopes =
        vec![normalize_target("src/gen", &base.canonical_root).expect("normalize")];
    let wave = vec![WaveClaim::new("item-a", ["src/declared.rs".to_string()])];
    let extended =
        plan_extended_to_unclaimed_changes(&base, &changed(&["src/gen/a.rs"]), Some(&wave));
    assert_eq!(declared(&extended), declared(&base));
}

/// A path the coordinator cannot name is never granted: normalisation failure
/// must drop it, so it meets `UndeclaredWrite` exactly as it does today.
#[test]
fn an_unnormalisable_path_is_never_granted() {
    let base = plan("item-a", &["src/declared.rs"]);
    let wave = vec![WaveClaim::new("item-a", ["src/declared.rs".to_string()])];
    let extended =
        plan_extended_to_unclaimed_changes(&base, &changed(&["../outside/escape.rs"]), Some(&wave));
    assert_eq!(declared(&extended), declared(&base));
}

// ---------------------------------------------------------------------------
// A granted path must carry a baseline, or the apply-time stale recheck skips
// it and the overlap guard stands alone between two items writing one file.
// ---------------------------------------------------------------------------

use crate::write_coordinator::worktree_isolation::{
    CanonicalBaseline, extend_baseline_with_granted_targets,
};

fn baseline_of(root: &std::path::Path, declared: &[&str]) -> CanonicalBaseline {
    let mut base = CanonicalBaseline {
        repo_fingerprint: "fp".into(),
        tracked_diff_binary: Vec::new(),
        untracked_files: Default::default(),
        declared_target_meta: Default::default(),
        verify_input_meta: Default::default(),
    };
    base = extend_baseline_with_granted_targets(
        &base,
        root,
        &declared
            .iter()
            .map(|p| (*p).to_string())
            .collect::<Vec<_>>(),
    );
    base
}

#[test]
fn a_granted_path_gains_a_baseline_hash() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/granted.rs"), "// content\n").expect("write");

    let base = baseline_of(dir.path(), &[]);
    assert!(base.declared_target_meta.is_empty());

    let extended =
        extend_baseline_with_granted_targets(&base, dir.path(), &["src/granted.rs".to_string()]);
    let meta = extended
        .declared_target_meta
        .get("src/granted.rs")
        .expect("granted path must be recorded, or the stale recheck skips it");
    assert!(meta.exists);
    assert!(!meta.blake3_hex.is_empty());
}

/// An existing baseline entry is authoritative: re-recording it would replace a
/// true baseline with the file as it is NOW, which is the opposite of the check.
#[test]
fn an_existing_baseline_entry_is_never_overwritten() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    let path = dir.path().join("src/declared.rs");
    std::fs::write(&path, "// original\n").expect("write");

    let base = baseline_of(dir.path(), &["src/declared.rs"]);
    let original = base.declared_target_meta["src/declared.rs"]
        .blake3_hex
        .clone();

    std::fs::write(&path, "// changed since\n").expect("rewrite");
    let extended =
        extend_baseline_with_granted_targets(&base, dir.path(), &["src/declared.rs".to_string()]);

    assert_eq!(
        extended.declared_target_meta["src/declared.rs"].blake3_hex, original,
        "re-recording would hide exactly the drift this check exists to find"
    );
}

/// A path that cannot be read is skipped, not failed: it then has no pre-hash,
/// which is where it started.
#[test]
fn an_unreadable_granted_path_is_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = baseline_of(dir.path(), &[]);
    let extended = extend_baseline_with_granted_targets(
        &base,
        dir.path(),
        &["src/never/created.rs".to_string()],
    );
    assert!(
        extended
            .declared_target_meta
            .get("src/never/created.rs")
            .is_none_or(|meta| !meta.exists),
        "an absent file must not be recorded as present"
    );
}
