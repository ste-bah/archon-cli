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
