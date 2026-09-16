//! The evidence rule, judged exactly as the preamble states it (Issue-14).
use super::*;
use crate::repository_audit::Verdict;

const SNAPSHOT: &str = "snap-1";

fn manifest(changed: &[&str], created: &[&str]) -> PatchManifest {
    PatchManifest {
        schema: "patch-manifest-v1".into(),
        run_id: "run".into(),
        stage_id: "stage".into(),
        item_id: "stage-0".into(),
        baseline_commit: "base".into(),
        patch_path: PathBuf::from("stage-0.patch"),
        declared_target_files: changed
            .iter()
            .chain(created)
            .map(|p| (*p).to_string())
            .collect(),
        changed_files: changed
            .iter()
            .chain(created)
            .map(|p| (*p).to_string())
            .collect(),
        created_files: created.iter().map(|p| (*p).to_string()).collect(),
        deleted_files: Vec::new(),
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::new(),
        verify_command: None,
        agent_artifact_path: None,
        status: ManifestStatus::PendingApply,
        skipped_ignored: BTreeMap::new(),
    }
}

fn record(declared: &str, equivalents: &[&str]) -> AuditRecord {
    AuditRecord {
        declared_path: declared.into(),
        verdict: Verdict::ExistsElsewhere,
        equivalents: equivalents.iter().map(|p| (*p).to_string()).collect(),
        required_action: RequiredAction::WireOrMigrate,
        reason: "exists at another path".into(),
    }
}

fn disposition(declared: &str, evidence: &[&str]) -> Disposition {
    Disposition {
        declared_path: declared.into(),
        snapshot: SNAPSHOT.into(),
        explanation: "created the declared module beside the existing one".into(),
        evidence_paths: evidence.iter().map(|p| (*p).to_string()).collect(),
    }
}

/// The live shape: the declared path (created), the audit's own equivalent
/// (unchanged, read), and one more created file. Every path stands.
#[test]
fn declared_path_equivalent_and_created_file_are_all_accepted() {
    let tree = tempfile::tempdir().unwrap();
    let manifest = manifest(&["src/lib.rs"], &["src/validation/checks.rs"]);
    let records = vec![record(
        "src/validation/checks.rs",
        &["src/data_store/validation/checks.rs"],
    )];
    let context = EvidenceContext {
        manifest: Some(&manifest),
        records: &records,
        worktree: tree.path(),
    };
    let judged = context
        .judge(
            disposition(
                "src/validation/checks.rs",
                &[
                    "src/validation/checks.rs",
                    "src/data_store/validation/checks.rs",
                    "src/lib.rs",
                ],
            ),
            SNAPSHOT,
        )
        .expect("the live disposition stands");
    assert!(judged.dropped.is_empty());
    assert_eq!(judged.disposition.evidence_paths.len(), 3);
}

/// Only unchanged paths: the equivalent and a file that exists in the tree.
/// Nothing anchors the disposition to this branch's work, so it does not stand.
#[test]
fn disposition_citing_only_unchanged_paths_is_rejected() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tree.path().join("docs")).unwrap();
    std::fs::write(tree.path().join("docs/design.md"), "read me").unwrap();
    let manifest = manifest(&["src/lib.rs"], &[]);
    let records = vec![record(
        "src/validation/checks.rs",
        &["src/data_store/validation/checks.rs"],
    )];
    let context = EvidenceContext {
        manifest: Some(&manifest),
        records: &records,
        worktree: tree.path(),
    };
    assert!(
        context
            .judge(
                disposition(
                    "src/validation/checks.rs",
                    &["src/data_store/validation/checks.rs", "docs/design.md"],
                ),
                SNAPSHOT,
            )
            .is_none()
    );
    // No manifest at all: nothing can anchor it either.
    let context = EvidenceContext {
        manifest: None,
        records: &records,
        worktree: tree.path(),
    };
    assert!(
        context
            .judge(
                disposition("src/validation/checks.rs", &["src/validation/checks.rs"]),
                SNAPSHOT
            )
            .is_none()
    );
}

/// One bogus path among valid ones is dropped and named; the rest stands.
#[test]
fn bogus_path_is_dropped_and_named_while_the_disposition_stands() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::write(tree.path().join("README.md"), "present").unwrap();
    let manifest = manifest(&[], &["src/validation/checks.rs"]);
    let records = vec![record("src/validation/checks.rs", &[])];
    let context = EvidenceContext {
        manifest: Some(&manifest),
        records: &records,
        worktree: tree.path(),
    };
    let judged = context
        .judge(
            disposition(
                "src/validation/checks.rs",
                &[
                    "src/validation/checks.rs",
                    "README.md",
                    "src/never/existed.rs",
                    "../outside.rs",
                    "/etc/hosts",
                ],
            ),
            SNAPSHOT,
        )
        .expect("stands on the created file");
    assert_eq!(
        judged.disposition.evidence_paths,
        vec!["src/validation/checks.rs", "README.md"]
    );
    assert_eq!(
        judged.dropped,
        vec!["src/never/existed.rs", "../outside.rs", "/etc/hosts"]
    );
    let note = dropped_note("src/validation/checks.rs", &judged.dropped);
    assert!(
        note.contains("src/never/existed.rs") && note.contains("dropped"),
        "{note}"
    );
}

/// Snapshot and explanation bounds are unchanged by the evidence rule.
#[test]
fn wrong_snapshot_or_bad_explanation_still_fails() {
    let tree = tempfile::tempdir().unwrap();
    let manifest = manifest(&[], &["a.rs"]);
    let records = vec![record("a.rs", &[])];
    let context = EvidenceContext {
        manifest: Some(&manifest),
        records: &records,
        worktree: tree.path(),
    };
    assert!(
        context
            .judge(disposition("a.rs", &["a.rs"]), "other-snapshot")
            .is_none()
    );
    let mut blank = disposition("a.rs", &["a.rs"]);
    blank.explanation = "   ".into();
    assert!(context.judge(blank, SNAPSHOT).is_none());
    let mut long = disposition("a.rs", &["a.rs"]);
    long.explanation = "x".repeat(2049);
    assert!(context.judge(long, SNAPSHOT).is_none());
}

/// The prompt states the same rule the check enforces, and names the fields.
#[test]
fn contract_text_states_the_evidence_rule_verbatim() {
    let text = contract_text();
    assert!(text.contains(AUDIT_EVIDENCE_RULE), "{text}");
    for field in [
        "declared_path",
        "snapshot",
        "explanation",
        "evidence_paths",
        "2048",
        "exactly one entry",
    ] {
        assert!(text.contains(field), "missing {field}: {text}");
    }
}

// ---------------------------------------------------------------------------
// Issue-15: a changed equivalent is judged by the scope grant, not by the
// assignment's declared list and never by the audit's mention of it.
// ---------------------------------------------------------------------------

use crate::v2::write_scope_extension::WaveClaim;
use crate::{WorkflowV2FileRecord, WorkflowV2Status};
use archon_write_plan::{TargetFilesSource, WritePlan, normalize_target};

const DECLARED: &str = "src/validation.rs";
const EQUIVALENT: &str = "src/data_store/validation.rs";

fn plan(targets: &[&str]) -> WritePlan {
    let canonical_root = PathBuf::from("/repo");
    WritePlan {
        run_id: "run".into(),
        stage_id: "stage".into(),
        item_id: "stage-0".into(),
        target_files: targets
            .iter()
            .map(|p| normalize_target(p, &canonical_root).unwrap())
            .collect(),
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "base".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
        isolated_root: PathBuf::from("/tmp/iso"),
        canonical_root,
    }
}

/// The grant as `run_one_worktree_branch` resolves it: this item declares
/// `DECLARED`, reports `reported`, and `other_claims` is what the one other
/// item in the wave owns.
fn grant(reported: &[&str], other_claims: &[&str]) -> ScopeGrant {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        files_changed: reported
            .iter()
            .map(|p| WorkflowV2FileRecord::new(*p))
            .collect(),
        ..Default::default()
    };
    let wave = vec![
        WaveClaim::new("stage-0", [DECLARED.to_string()]),
        WaveClaim::new("stage-1", other_claims.iter().map(|p| (*p).to_string())),
    ];
    ScopeGrant::resolve_unforbidden(&plan(&[DECLARED]), &result, Some(&wave))
}

fn report() -> AuditReport {
    AuditReport {
        schema_version: 1,
        snapshot: SNAPSHOT.into(),
        records: vec![record(DECLARED, &[EQUIVALENT])],
    }
}

/// The live shape: the branch created the declared file and migrated code
/// out of the equivalent, reported both, and cited both.
fn judge_migration(grant: &ScopeGrant) -> AuditJudgement {
    let tree = tempfile::tempdir().unwrap();
    let manifest = manifest(&[EQUIVALENT], &[DECLARED]);
    let report = report();
    let context = EvidenceContext {
        manifest: Some(&manifest),
        records: &report.records,
        worktree: tree.path(),
    };
    judge_branch(
        &report,
        &|_| false,
        &[disposition(DECLARED, &[DECLARED, EQUIVALENT])],
        &context,
        &[DECLARED.to_string()],
        grant,
    )
}

/// (a) The equivalent is unclaimed by the other item, so it was granted: the
/// branch stands and the reviewer is told which finding the change answers.
#[test]
fn granted_equivalent_is_authorised_and_named_for_review() {
    let grant = grant(&[DECLARED, EQUIVALENT], &["src/other.rs"]);
    assert_eq!(grant.granted, vec![EQUIVALENT.to_string()], "precondition");
    let judgement = judge_migration(&grant);
    assert!(judgement.gaps.is_empty(), "{:?}", judgement.gaps);
    let note = judgement
        .notes
        .iter()
        .find(|n| n.contains(EQUIVALENT))
        .unwrap_or_else(|| panic!("{:?}", judgement.notes));
    assert!(
        note.contains(&format!("audit finding for {DECLARED}"))
            && note.contains("scope grant")
            && note.contains("scope_granted"),
        "{note}"
    );
}

/// (b) The other item claims the equivalent, so it is contested and not
/// granted: rejected with the same message as before.
#[test]
fn contested_equivalent_is_still_rejected() {
    let grant = grant(&[DECLARED, EQUIVALENT], &[EQUIVALENT]);
    assert!(grant.granted.is_empty(), "precondition");
    let judgement = judge_migration(&grant);
    assert_eq!(
        judgement.gaps,
        vec![format!(
            "{EQUIVALENT} (equivalent is outside declared ownership)"
        )]
    );
    assert!(judgement.notes.is_empty(), "{:?}", judgement.notes);
}

/// (c) The equivalent was changed but never reported, so nothing granted it —
/// the audit's mention of it does not: rejected as before.
#[test]
fn unreported_equivalent_is_not_granted_by_the_audit_mention() {
    let grant = grant(&[DECLARED], &["src/other.rs"]);
    assert!(grant.granted.is_empty(), "precondition");
    let judgement = judge_migration(&grant);
    assert_eq!(
        judgement.gaps,
        vec![format!(
            "{EQUIVALENT} (equivalent is outside declared ownership)"
        )]
    );
}

/// An unchanged equivalent — the ordinary case — is neither a gap nor a note,
/// whatever the grant says.
#[test]
fn unchanged_equivalent_is_not_judged() {
    let tree = tempfile::tempdir().unwrap();
    let manifest = manifest(&[], &[DECLARED]);
    let report = report();
    let context = EvidenceContext {
        manifest: Some(&manifest),
        records: &report.records,
        worktree: tree.path(),
    };
    let judgement = judge_branch(
        &report,
        &|_| false,
        &[disposition(DECLARED, &[DECLARED, EQUIVALENT])],
        &context,
        &[DECLARED.to_string()],
        &grant(&[DECLARED], &[]),
    );
    assert!(
        judgement.gaps.is_empty() && judgement.notes.is_empty(),
        "{:?} {:?}",
        judgement.gaps,
        judgement.notes
    );
}
