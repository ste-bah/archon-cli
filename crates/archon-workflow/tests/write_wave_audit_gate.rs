//! Issue-14: the audit-gate prompt and check are one contract.
//!
//! Live on wf-7db01ce7 `agents-3-0`, a branch whose work was complete
//! returned a correct disposition for an `exists_elsewhere` finding, citing
//! the declared path it had created and the audit's own equivalent (unchanged,
//! as it must be). The check required every evidence path to be a changed
//! file, voided the disposition, failed the manifest, and every dependent
//! wave was skipped. These drive the production write-wave seam with a
//! scripted audit and prove the prompt's rule is the check's rule, that a
//! rejected branch keeps its work, and that the next attempt at the same
//! task starts from it.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::v2::write::AUDIT_EVIDENCE_RULE;
use archon_workflow::*;
use std::collections::BTreeMap;
use support::{AuditScript, Edits, Fixture, git};

/// `added.txt` is flagged as existing elsewhere (`other.txt`, unchanged).
fn flagged() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![("added.txt", vec!["other.txt"])]
}

fn script(branch: &str, evidence: Vec<&'static str>) -> AuditScript {
    let mut dispositions = BTreeMap::new();
    dispositions.insert(branch.to_string(), vec![("added.txt", evidence)]);
    AuditScript {
        flagged: flagged(),
        dispositions,
    }
}

fn live_edits() -> Edits {
    Edits {
        files: vec![("added.txt", "created here\n"), ("owned.txt", "wired\n")],
        report: vec!["added.txt", "owned.txt"],
        via_adapter: true,
    }
}

/// (a) + (f): the live disposition — declared path, equivalent, one more
/// changed file — is accepted, and the prompt stated the rule it was judged by.
#[tokio::test]
async fn disposition_citing_declared_path_equivalent_and_changed_file_is_accepted() {
    let f = Fixture::new();
    let (out, prompts) = f
        .wave_audited(
            "live",
            vec![(vec!["added.txt", "owned.txt"], live_edits())],
            Some(script(
                "live-0",
                vec!["added.txt", "other.txt", "owned.txt"],
            )),
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:added.txt"]), "created here");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "wired");
    let result = f.branch_result("live", "live-0");
    assert!(
        !result
            .residual_gaps
            .iter()
            .any(|g| g.id == "repository_audit_unaddressed"),
        "{result:#?}"
    );
    assert!(
        !result
            .evidence
            .iter()
            .any(|e| e.summary.contains("evidence path(s) dropped")),
        "{result:#?}"
    );
    assert_eq!(prompts.len(), 1, "{prompts:#?}");
    assert!(prompts[0].contains(AUDIT_EVIDENCE_RULE), "{}", prompts[0]);
    assert!(
        prompts[0].contains("exactly one entry in data.audit_dispositions"),
        "{}",
        prompts[0]
    );
    assert!(
        prompts[0].contains("\"declared_path\":\"added.txt\""),
        "{}",
        prompts[0]
    );
}

/// (b) + (d): a disposition citing only unchanged paths is rejected — and the
/// branch's complete work is kept as partial work, which the next attempt at
/// the same task resumes from and lands.
#[tokio::test]
async fn rejected_disposition_keeps_the_work_for_the_next_attempt_at_the_task() {
    let f = Fixture::new();
    let (out, _) = f
        .wave_audited(
            "reject",
            vec![(vec!["added.txt", "owned.txt"], live_edits())],
            Some(script("reject-0", vec!["other.txt", "src/formatted.txt"])),
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["rev-parse", "HEAD"]),
        f.base,
        "nothing may land"
    );
    let result = f.branch_result("reject", "reject-0");
    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    let gap = result
        .residual_gaps
        .iter()
        .find(|g| g.id == "repository_audit_unaddressed")
        .unwrap_or_else(|| panic!("{result:#?}"));
    assert!(gap.description.contains("added.txt"), "{gap:?}");
    assert_eq!(
        f.manifest("reject", "reject-0")["status"]["status"],
        "failed"
    );
    let partial = &result.data["partial_work"];
    let files: Vec<&str> = partial["files"]
        .as_array()
        .unwrap_or_else(|| panic!("partial work not captured: {result:#?}"))
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(files, vec!["added.txt", "owned.txt"], "{partial}");
    assert!(std::path::Path::new(partial["patch_path"].as_str().unwrap()).is_file());
    assert_eq!(
        result.data["canonical_task_ids"],
        serde_json::json!(["TASK-001"])
    );

    // The next attempt at TASK-001 writes nothing itself: what lands is the
    // first attempt's work, applied into its fresh worktree by the host.
    let (out, prompts) = f
        .wave_audited(
            "resume",
            vec![(
                vec!["added.txt", "owned.txt"],
                Edits {
                    files: vec![],
                    report: vec!["added.txt", "owned.txt"],
                    via_adapter: true,
                },
            )],
            Some(script("resume-0", vec!["added.txt", "other.txt"])),
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:added.txt"]), "created here");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "wired");
    let prompt = &prompts[0];
    assert!(
        prompt.contains("Its uncommitted work (2 file(s)) has been applied to this workspace: added.txt, owned.txt"),
        "{prompt}"
    );
}

/// (c) one bogus path among valid ones: dropped, named, disposition stands.
#[tokio::test]
async fn bogus_evidence_path_is_dropped_and_named_but_does_not_void_the_disposition() {
    let f = Fixture::new();
    let (out, _) = f
        .wave_audited(
            "bogus",
            vec![(vec!["added.txt", "owned.txt"], live_edits())],
            Some(script("bogus-0", vec!["added.txt", "no/such/file.txt"])),
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:added.txt"]), "created here");
    let result = f.branch_result("bogus", "bogus-0");
    let note = result
        .evidence
        .iter()
        .find(|e| e.summary.contains("evidence path(s) dropped"))
        .unwrap_or_else(|| panic!("{result:#?}"));
    assert!(
        note.summary.contains("audit disposition for added.txt")
            && note.summary.ends_with("no/such/file.txt"),
        "{note:?}"
    );
}

/// (e) `patch_landed` is judged against the granted plan: a branch whose only
/// change is a granted unclaimed file has landed.
#[tokio::test]
async fn patch_landed_is_true_for_a_branch_whose_only_change_is_a_granted_file() {
    let f = Fixture::new();
    let out = f
        .wave(
            "granted-only",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![("forgotten.txt", "only this\n")],
                    report: vec!["forgotten.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:forgotten.txt"]), "only this");
    let result = f.branch_result("granted-only", "granted-only-0");
    assert_eq!(
        result.data["scope_granted"],
        serde_json::json!(["forgotten.txt"]),
        "{result:#?}"
    );
    assert_eq!(
        result.data["patch_landed"],
        serde_json::json!(true),
        "{result:#?}"
    );
}
