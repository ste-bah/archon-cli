//! Issue-76: the host's own bookkeeping never lands, and is never advertised
//! to a coder as one of the branch's artifacts.
//!
//! Live on `wf-0ddadd81`: the branch result carried the host's write manifest
//! on its `artifacts` list, the replayed rejected-attempt envelope rendered
//! that entry into the coder's prompt, and the coder copied the host's file
//! into its worktree at the repository root — where the scope-roots drop
//! exempts root-level paths, nothing in the wave contested it, and it was
//! granted, declared and committed into the target repository. This drives the
//! production write-wave seam with real Git writes over both layers.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

/// The basename the live coder reproduced inside its worktree.
const HOST_FILE: &str = "patch_manifest.json";

/// The item declares `owned.txt`, implements it, and also writes a copy of the
/// host's write manifest at the repository root, reporting both. The real work
/// lands; the host file never reaches the canonical tree, the manifest or the
/// envelope; the branch is accepted, with a review gap naming the path.
#[tokio::test]
async fn a_host_bookkeeping_file_is_dropped_and_the_real_work_lands() {
    let f = Fixture::new();
    let out = f
        .wave(
            "host",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        (
                            HOST_FILE,
                            "{\"item_id\":\"item\",\"stage\":\"R09\",\"write_mode\":\"worktree\"}\n",
                        ),
                    ],
                    report: vec!["owned.txt", HOST_FILE],
                    via_adapter: true,
                },
            )],
        )
        .await;

    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert!(
        !f.repo.join(HOST_FILE).exists(),
        "the host's own bookkeeping must never reach the target repository"
    );
    assert_eq!(
        git(
            &f.repo,
            &["status", "--porcelain", "--", "owned.txt", HOST_FILE]
        ),
        ""
    );
    let manifest = f.manifest("host", "host-0");
    assert_eq!(manifest["changed_files"], json!(["owned.txt"]));
    assert_eq!(manifest["declared_target_files"], json!(["owned.txt"]));

    let result = f.branch_result("host", "host-0");
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(result.data["patch_landed"], json!(true), "{result:#?}");
    assert_eq!(result.data["host_internal_dropped"], json!([HOST_FILE]));
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == "host_internal_artifact_dropped_host-0")
        .unwrap_or_else(|| panic!("no host-internal gap: {result:#?}"));
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(gap.description.contains(HOST_FILE), "{gap:?}");
    assert!(
        !result
            .files_changed
            .iter()
            .any(|file| file.path.contains(HOST_FILE)),
        "the dropped path must leave the envelope too: {result:#?}"
    );
}

/// Layer 1: the envelope a rejected attempt replays verbatim into the next
/// coder's prompt names no host coordination path at all — the write manifest
/// used to ride on `artifacts`, which is what the coder reproduced.
#[tokio::test]
async fn a_branch_envelope_advertises_no_host_coordination_path() {
    let f = Fixture::new();
    let out = f
        .wave(
            "plain",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![("owned.txt", "implemented\n")],
                    report: vec!["owned.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");

    let result = f.branch_result("plain", "plain-0");
    let rendered = serde_json::to_string(&result).unwrap();

    assert!(
        !rendered.contains("write-coordination"),
        "the host's coordination directory must never reach a prompt: {rendered}"
    );
    assert!(
        !rendered.contains("/manifests/"),
        "the host's manifest path must never reach a prompt: {rendered}"
    );
    assert!(
        !result
            .artifacts
            .iter()
            .any(|artifact| artifact.id.starts_with("patch_manifest")),
        "the host's manifest is not a branch artifact: {result:#?}"
    );
}
