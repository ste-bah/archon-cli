//! Which acceptance-policy findings go back to the author.
//!
//! A real PRD froze all eleven of its acceptance checks as floors that could
//! not fail, and not one finding reached the author, because every policy
//! finding was scoped as inherited on the premise that "the PRD may mandate
//! the shape". Only a PRD that names the check's shape in the engine's own
//! vocabulary mandates it; the rest leave the shape to the author, and the
//! author is the one who can fix it.

use super::*;

/// Seed a PRD whose one criterion reads as given, and a draft contract whose
/// check is too weak to be falsifiable, so the freeze reports policy findings.
fn seed_with_criterion(temp: &tempfile::TempDir, criterion: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let prd = temp.path().join("prds/PRD-X.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::write(
        &prd,
        format!("## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | {criterion} |\n"),
    )
    .unwrap();
    let contract = r#"{
      "schema_version":1,
      "prd":{"path":"prds/PRD-X.md","digest":"pending"},
      "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
      "acceptance":[{
        "id":"AC-X-001","criterion":"untrusted draft summary",
        "check":{"kind":"command","command":"true","cwd":"project_root"},
        "gap_permitted":false,
        "judgment":{"verdict":"refuted","counterexample":"untrusted","reason":"untrusted","host_call_id":"untrusted"}
      }],
      "supplementary":[]
    }"#;
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), contract).unwrap();
    (tasks, prd)
}

async fn scopes_for(criterion: &str) -> Vec<archon_workflow::RemediationScope> {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd) = seed_with_criterion(&temp, criterion);
    let prepared = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"accepted for shadow measurement"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    assert!(
        !prepared.findings.is_empty(),
        "a check that cannot fail must produce policy findings"
    );
    prepared.findings.iter().map(|finding| finding.remediation_scope.clone()).collect()
}

/// Outcome language: the author chose the unfalsifiable shape, so the finding
/// is the author's to repair and the decomposition script retries on it.
#[tokio::test]
async fn a_criterion_in_outcome_language_makes_its_findings_repairable() {
    let scopes = scopes_for("`status` shows the existing project data root and registry.").await;
    assert!(
        scopes.iter().all(|scope| *scope == archon_workflow::RemediationScope::CandidateArtifact),
        "{scopes:?}"
    );
}

/// The PRD fixed the shape itself: the same finding is an observation about
/// the input, recorded and never retried, or the author would be sent to
/// violate the PRD.
#[tokio::test]
async fn a_criterion_prescribing_the_check_shape_keeps_its_findings_inherited() {
    let scopes = scopes_for(
        "The artifact exists. Freeze this criterion as a commandless floor with `artifact_path=\"x.json\"` and no `typed_verifier_command`.",
    )
    .await;
    assert!(
        scopes.iter().all(|scope| *scope == archon_workflow::RemediationScope::InheritedPredecessor),
        "{scopes:?}"
    );
}
