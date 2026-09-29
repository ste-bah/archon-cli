//! Batch J end to end, as the acceptance stage runs it: the regression
//! search, then routing, then the blocked classification, over a real
//! repository whose run landings reproduce wf-0ddadd81. The check held at
//! the run base; task X's landing broke it in a file no task declares; the
//! owner Y forbids that directory and landed after the break. The failure
//! prints no `path:line`. The finding must reach X with the file granted.

use std::collections::BTreeMap;

use super::tests::{World, git};
use super::{
    CheckObserver, FailingCheck, SearchBudget, Verdict, command_fingerprint, failure_signature,
};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::acceptance_routing::{mark_blocked, route_failures};
use crate::v2::acceptance_stage::{
    AcceptanceCheckRecordV1, AcceptanceCheckStatus, AcceptanceRoundRecordV1,
};
use crate::write_coordinator::worktree_isolation::run_git;

/// The check passes while the ingest module accepts an unknown class.
struct IngestObserver(std::path::PathBuf);

#[async_trait::async_trait]
impl CheckObserver for IngestObserver {
    async fn observe(&self, commit: &str, ids: &[String]) -> Option<BTreeMap<String, Verdict>> {
        let source = run_git(&["show", &format!("{commit}:pkg/core/ingest.rs")], &self.0)
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        let passed = !source.contains("reject unknown");
        Some(
            ids.iter()
                .map(|id| (id.clone(), Verdict::Held(passed)))
                .collect(),
        )
    }
}

fn task(
    id: &str,
    owns: &[&str],
    forbids: &[&str],
    implements: &[&str],
) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        implements: implements.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_regression_in_an_undeclared_file_reaches_its_author_with_the_file_granted() {
    let w = World::new(&[]);
    w.step("pkg/core.rs", "mod ingest;", "implement-x-1", "TASK-X");
    w.step(
        "pkg/core/ingest.rs",
        "accept all",
        "implement-x-2",
        "TASK-X",
    );
    w.step("pkg/cli/args.rs", "v1", "implement-y-3", "TASK-Y");
    let breaking = w.step(
        "pkg/core/ingest.rs",
        "reject unknown",
        "review-remediate-task-x-1-4",
        "TASK-X",
    );
    w.step(
        "pkg/cli/args.rs",
        "v2",
        "review-remediate-task-y-1-5",
        "TASK-Y",
    );
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            // Declares the module, not the file the break is in.
            task("TASK-X", &["pkg/core.rs"], &[], &[]),
            // Owns the check and may not touch the core directory.
            task(
                "TASK-Y",
                &["pkg/cli/args.rs"],
                &["`pkg/core/**`"],
                &["AC-1"],
            ),
        ],
    };
    let stderr =
        "warning: unused\n  = note: on by default\n\nError: unknown asset_class `unknown`\n";
    let mut record = AcceptanceRoundRecordV1 {
        schema_version: 1,
        run_id: "run".into(),
        call_id: "acceptance-contract-run-1".into(),
        round: 1,
        attempt: 1,
        max_rounds: 3,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks: vec![AcceptanceCheckRecordV1 {
            check_id: "AC-1".into(),
            criterion: "an unknown class is ingested".into(),
            kind: "command".into(),
            status: AcceptanceCheckStatus::Failed,
            exit_code: Some(1),
            operational_error: None,
            owning_tasks: vec!["TASK-Y".into()],
            stdout_tail: String::new(),
            stderr_tail: stderr.into(),
            regressed_by: None,
            contract_defect: false,
            routing: None,
            regression_search: None,
            blocked: None,
        }],
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: false,
    };
    // Without the search the check routes to Y alone, who cannot write it.
    route_failures(Some(&universe), &w.repo, &[], &mut record);
    assert_eq!(record.checks[0].routing, None);
    // The stage's order: search, route, classify.
    let failing = [FailingCheck {
        id: "AC-1".into(),
        owners: vec!["TASK-Y".into()],
        signature: failure_signature(Some(1), stderr, ""),
        fingerprint: command_fingerprint("archon ingest --asset-class unknown"),
    }];
    let found = w
        .attribute(
            &failing,
            &IngestObserver(w.repo.clone()),
            SearchBudget::default(),
        )
        .await;
    let check = &mut record.checks[0];
    check.regressed_by = found.regressions.get("AC-1").cloned();
    check.regression_search = found.searches.get("AC-1").cloned();
    route_failures(Some(&universe), &w.repo, &[], &mut record);
    mark_blocked(&mut record);
    let check = &record.checks[0];
    let regression = check.regressed_by.as_ref().expect("attributed");
    assert_eq!(regression.landing_commit, breaking);
    assert_eq!(regression.tasks, ["TASK-X"]);
    assert_eq!(
        regression.held_at,
        git(&w.repo, &["rev-parse", &format!("{breaking}^")])
    );
    let routing = check.routing.as_ref().expect("routed");
    assert_eq!(routing.granted_files, ["pkg/core/ingest.rs"], "{routing:?}");
    assert!(routing.unwritable.is_empty(), "{routing:?}");
    assert_eq!(check.blocked, None);
    assert!(record.has_remediable_failures());
}
