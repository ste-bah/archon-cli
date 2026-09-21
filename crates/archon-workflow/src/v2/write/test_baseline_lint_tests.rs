//! Issue-64: a declared lint/format/build command red on the base commit for
//! diagnostics in files outside the task's target set is recorded as
//! pre-existing — the coder is told, the verifier does not blame it — while
//! a diagnostic in one of the task's own files still counts.
use std::collections::BTreeMap;

use super::super::test_baseline_preamble::preamble;
use super::super::test_baseline_wave::{WaveBaselineContext, establish_wave};
use super::tests::{Host, head, repository, request, universe};
use crate::v2::verification::baseline_rule::{BaselineStamp, enforce_baseline_tests};
use crate::v2::{
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2ResultStore, WorkflowV2Status,
};

const LINT: &str = "cargo clippy -p app --lib -- -D warnings";

/// A declared lint whose base-commit output carries error diagnostics in
/// `files` and a warning (not a failure) in `src/mine.rs`.
fn lint_command(files: &[&str]) -> String {
    let mut output = String::from("warning: unused variable: `w`\\n --> src/mine.rs:3:1\\n  |\\n");
    for (index, file) in files.iter().enumerate() {
        output.push_str(&format!(
            "error: unused import: `x{index}`\\n --> {file}:{}:5\\n  |\\n",
            index + 1
        ));
    }
    output.push_str("error: could not compile `app` due to previous errors\\n");
    format!(": {LINT} ; printf '{output}'; exit 101")
}

fn verifier_outcome(command: &str, output: &str) -> WorkflowV2BranchOutcome {
    let mut result = crate::WorkflowV2Result::accepted("verified");
    result.commands_run = vec![WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: command.into(),
        status: WorkflowV2CommandStatus::Failed,
        exit_code: Some(101),
        output_summary: output.into(),
        pre_existing: true,
    }];
    WorkflowV2BranchOutcome {
        item_id: "verify-1-check".into(),
        role: "coder".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

#[tokio::test]
async fn out_of_scope_diagnostics_are_pre_existing_and_an_in_scope_one_is_still_owed() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, ws) = repository(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let base = head(&canonical);
    let universe = universe();
    let ctx = WaveBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: Some(&universe),
        stage_id: "agents-5",
        base_commit: &base,
        parallelism: 1,
    };
    // Every error is outside TASK-A's scope: one in TASK-B's file, one in
    // nobody's. The warning in its own file is not a failure.
    let outside = lint_command(&["src/theirs.rs", "src/nobody.rs"]);
    let records = establish_wave(
        &ctx,
        &[request(
            "agents-5-a",
            "TASK-A",
            &outside,
            &ws,
            &["src/mine.rs"],
        )],
    )
    .await;
    let record = &records[0];
    assert_eq!(record.commands[0].exit_code, Some(101));
    assert_eq!(
        record.commands[0].diagnostic_files,
        vec!["src/nobody.rs".to_string(), "src/theirs.rs".to_string()]
    );
    assert!(record.obligations.is_empty(), "{:?}", record.obligations);
    assert!(record.obligation_files().is_empty());
    assert!(record.must_pass().is_empty());
    assert_eq!(record.pre_existing.len(), 1);
    assert_eq!(record.pre_existing[0].command, outside);
    assert_eq!(
        record.pre_existing[0].files,
        vec!["src/nobody.rs".to_string(), "src/theirs.rs".to_string()]
    );
    assert_eq!(
        record.pre_existing[0].owners,
        vec![("src/theirs.rs".to_string(), "TASK-B".to_string())]
    );
    let text = preamble(record);
    assert!(
        text.contains(&format!(
            "- `{outside}` already fails at the base commit in 2 file(s) outside your \
             target_files: src/nobody.rs (unowned), src/theirs.rs (owned by TASK-B); do not fix \
             them — they are reported as pre-existing; you are held only to diagnostics in your \
             own files.\n"
        )),
        "{text}"
    );
    assert!(
        text.contains("a write there is refused and would be dropped from your patch"),
        "{text}"
    );

    // The verifier's stamp carries the command and its files; a pre_existing
    // claim whose output locates only those files keeps the verdict, one
    // that locates the task's own file does not.
    let stamp = BaselineStamp::for_tasks(&store, &["TASK-A".to_string()]).expect("stamp");
    assert_eq!(stamp.pre_existing_diagnostics.len(), 1);
    assert_eq!(stamp.pre_existing_diagnostics[0].command, outside);
    assert_eq!(
        stamp.pre_existing_diagnostics[0].files,
        vec!["src/nobody.rs".to_string(), "src/theirs.rs".to_string()]
    );
    assert!(stamp.must_pass.is_empty());
    let by_item = BTreeMap::from([("verify-1-check".to_string(), stamp)]);
    let mut kept = vec![verifier_outcome(
        &outside,
        "error: unused import: `x0`\n --> src/theirs.rs:1:5\nerror: unused import: `x1`\n \
         --> src/nobody.rs:2:5\nfails identically on the base commit",
    )];
    enforce_baseline_tests(&mut kept, &by_item);
    assert_eq!(
        kept[0].status,
        WorkflowV2Status::Accepted,
        "{:?}",
        kept[0].result
    );
    let mut bare = vec![verifier_outcome(
        &outside,
        "fails identically on the base commit",
    )];
    enforce_baseline_tests(&mut bare, &by_item);
    assert_eq!(bare[0].status, WorkflowV2Status::Accepted);
    let mut blamed = vec![verifier_outcome(
        &outside,
        "error: unused import: `x0`\n --> src/theirs.rs:1:5\nerror[E0308]: mismatched types\n \
         --> src/mine.rs:9:1\n",
    )];
    enforce_baseline_tests(&mut blamed, &by_item);
    assert_eq!(blamed[0].status, WorkflowV2Status::NeedsReview);
    let result = blamed[0].result.as_ref().unwrap();
    assert_eq!(
        result.data["baseline_unproven_pre_existing"],
        serde_json::json!([outside])
    );

    // A diagnostic in the task's own file on the base commit is its
    // obligation, named by file; the out-of-scope one stays pre-existing.
    let mixed = lint_command(&["src/mine.rs", "src/theirs.rs"]);
    let ctx = WaveBaselineContext {
        stage_id: "agents-6",
        ..ctx
    };
    let records = establish_wave(
        &ctx,
        &[request(
            "agents-6-a",
            "TASK-A",
            &mixed,
            &ws,
            &["src/mine.rs"],
        )],
    )
    .await;
    let record = &records[0];
    assert_eq!(record.obligations.len(), 1);
    assert_eq!(record.obligations[0].test_id, None);
    assert_eq!(record.obligations[0].file.as_deref(), Some("src/mine.rs"));
    assert_eq!(record.pre_existing.len(), 1);
    assert_eq!(
        record.pre_existing[0].files,
        vec!["src/theirs.rs".to_string()]
    );
    let text = preamble(record);
    assert!(
        text.contains(&format!(
            "- Tests already failing on the base commit within your declared filter: `{mixed}` \
             reports error diagnostics in src/mine.rs — these are yours to make pass"
        )),
        "{text}"
    );
    assert!(
        text.contains("in 1 file(s) outside your target_files: src/theirs.rs (owned by TASK-B)"),
        "{text}"
    );

    // A failure with no location at all is the task's, as before.
    let located_nowhere = format!(": {LINT} ; echo 'linker failed'; exit 1");
    let ctx = WaveBaselineContext {
        stage_id: "agents-7",
        ..ctx
    };
    let records = establish_wave(
        &ctx,
        &[request(
            "agents-7-a",
            "TASK-A",
            &located_nowhere,
            &ws,
            &["src/mine.rs"],
        )],
    )
    .await;
    assert_eq!(records[0].obligations.len(), 1);
    assert_eq!(records[0].obligations[0].file, None);
    assert!(records[0].pre_existing.is_empty());
}
