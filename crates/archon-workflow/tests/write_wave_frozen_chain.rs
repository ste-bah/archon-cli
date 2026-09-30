//! Batch O (I11): an agent landing that edits the frozen acceptance chain
//! is rejected on the host, even when the file is the branch's declared
//! target. Only a recorded freeze or re-author/republish changes the
//! contract, the skeleton, their locks or the pin store.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

const CONTRACT: &str = "tasks/SET/acceptance-contract.json";

#[tokio::test]
async fn a_landing_that_edits_the_frozen_contract_is_rejected_even_when_declared() {
    let mut f = Fixture::new();
    let contract = f.repo.join(CONTRACT);
    std::fs::create_dir_all(contract.parent().unwrap()).unwrap();
    std::fs::write(&contract, "{\"frozen\": true}\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "frozen chain"]);
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-001".into(),
            source_path: "tasks/TASK-001.md".into(),
            files_expected_to_change: vec![
                "`owned.txt` — exists (1 lines)".into(),
                format!("`{CONTRACT}` — exists (1 lines)"),
            ],
            ..Default::default()
        }],
    });
    let (out, _) = f
        .wave_audited(
            "frozen",
            vec![(
                vec!["owned.txt", CONTRACT],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        (CONTRACT, "{\"frozen\": false}\n"),
                    ],
                    report: vec!["owned.txt", CONTRACT],
                    via_adapter: true,
                },
            )],
            None,
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let result = f.branch_result("frozen", "frozen-0");
    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    assert_eq!(result.data["forbidden_paths_changed"], json!([CONTRACT]));
    assert_eq!(result.data["patch_landed"], json!(false), "{result:#?}");
    assert_eq!(
        git(&f.repo, &["show", &format!("HEAD:{CONTRACT}")]),
        "{\"frozen\": true}"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "baseline");
}
