use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::runner::{PipelineFacade, run_pipeline_audited};

use super::{FINDINGS, PAPER};
use super::{ValidatorFacade, llm};
use archon_pipeline::research::draft_artifact::write_draft_research_artifacts;

#[tokio::test]
async fn failed_final_export_keeps_draft_artifacts_at_active_names() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-export-failure".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = llm(true);
    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir("repair-export-failure");
    let obstruction = bundle.join("exports/final-paper.md");
    assert!(
        run_pipeline_audited(
            &facade as &dyn PipelineFacade,
            llm.as_ref(),
            &format!("block-final-export={}", obstruction.display()),
            tmp.path(),
            None,
            None,
            None,
        )
        .await
        .is_err()
    );
    assert!(bundle.join("exports/draft-paper.md").is_file());
    assert!(bundle.join("exports/draft-paper.findings.md").is_file());
    assert!(!bundle.join("exports/superseded-draft-paper.md").exists());
    assert!(
        !bundle
            .join("exports/superseded-draft-paper.findings.md")
            .exists()
    );
}

#[tokio::test]
async fn audit_complete_failure_keeps_draft_artifacts_active() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "repair-audit-complete-failure";
    let facade = ValidatorFacade {
        session_id: session_id.into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir(session_id);
    write_draft_research_artifacts(&bundle, PAPER, FINDINGS).unwrap();

    let result = run_pipeline_audited(
        &facade as &dyn PipelineFacade,
        llm(false).as_ref(),
        &format!(
            "block-audit-complete={}",
            bundle.join("audit.log").display()
        ),
        tmp.path(),
        None,
        None,
        None,
    )
    .await;

    assert!(result.is_err(), "audit completion should fail");
    assert!(bundle.join("exports/draft-paper.md").is_file());
    assert!(bundle.join("exports/draft-paper.findings.md").is_file());
    assert!(!bundle.join("exports/superseded-draft-paper.md").exists());
    assert!(
        !bundle
            .join("exports/superseded-draft-paper.findings.md")
            .exists()
    );
}
