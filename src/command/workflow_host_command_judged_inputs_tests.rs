//! Issue 337 round 5 (review finding 3): an unpublished outcome of a gate
//! that takes a candidate replays only while the task-root content it judged
//! is unchanged. Its call identity binds the candidate, the PRD digest and
//! the paths, not that content, so the content digest is stamped on the
//! outcome and compared again at replay.

use std::sync::Arc;

use archon_workflow::{HostCommandRequest, WorkflowStore, WorkflowV2CallRecord};

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, fixed_decomposition_catalog,
};
use super::workflow_host_command_exec::{FixedHostCommandExecutor, WorkflowHostCommandExecutor};
use super::workflow_host_command_exec_tests::{PreparedBodyProcess, context, seed_frozen_chain};
use super::workflow_host_command_judged_inputs::{JUDGED_INPUTS, answers_current_inputs, stamp};
use super::workflow_host_command_terminal_stop_tests::CANDIDATE;

struct Fixture {
    _temp: tempfile::TempDir,
    context: HostCommandResolutionContext,
    executor: FixedHostCommandExecutor,
    run_id: String,
}

/// A task root with the frozen chain and one frozen body, TASK-X-010.md.
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    // A PRD with obligations: the acceptance freeze binds the launch PRD.
    std::fs::write(
        &context.prd_path,
        "# PRD X\n\n## Requirements\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Initial |\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | Initial |\n",
    )
    .unwrap();
    context.prd_digest = super::workflow_task_set::validate_prd_input(&context.prd_path)
        .unwrap()
        .1;
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&context, &task_file);
    let store = WorkflowStore::project(&context.project_root);
    let run_id = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "judged-inputs".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap()
        .id;
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context.clone(),
        store.run_dir(&run_id),
        Arc::new(PreparedBodyProcess {
            candidate: CANDIDATE.as_bytes().to_vec(),
        }),
    );
    Fixture {
        _temp: temp,
        context,
        executor,
        run_id,
    }
}

impl Fixture {
    /// The unpublished outcome of `request` as the host records it now:
    /// filed under its identity, stamped with what it judged (if anything).
    fn record(&self, request: &HostCommandRequest) -> WorkflowV2CallRecord {
        let mut options = archon_workflow::WorkflowV2HostOptions::default();
        options.host_command = Some(request.clone());
        let mut result = archon_workflow::WorkflowV2Result::default();
        result.data = serde_json::json!({ "exitCode": 1 });
        let judged = self.executor.judged_inputs(request).unwrap();
        stamp(&mut result.data, judged.clone(), judged);
        WorkflowV2CallRecord::new(
            &self.run_id,
            archon_workflow::WorkflowV2HostCall {
                id: self.executor.call_identity(request).unwrap(),
                method: archon_workflow::WorkflowV2HostMethod::HostCommand,
                write_mode: None,
                options,
            },
            1,
            "input".into(),
            result,
            Vec::new(),
        )
    }

    fn answers(&self, record: &WorkflowV2CallRecord) -> bool {
        answers_current_inputs(&self.executor, record).unwrap()
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.context.task_root.join(name), bytes).unwrap();
    }
}

/// The reviewer's case: a freeze refusal that the task root caused; the
/// operator repairs the task root. The stale refusal no longer answers; the
/// same content again answers again.
#[test]
fn a_freeze_refusal_answers_only_while_the_task_root_it_judged_is_unchanged() {
    use archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    for command in ["freeze-acceptance", "freeze-skeleton"] {
        let fixture = fixture();
        let request = HostCommandRequest::new(command, Some("candidate".into())).unwrap();
        let record = fixture.record(&request);
        assert!(record.result.data[JUDGED_INPUTS].is_string(), "{command}");
        assert!(fixture.answers(&record), "{command}: nothing changed");

        let original = std::fs::read(fixture.context.task_root.join(ACCEPTANCE_CONTRACT_FILE));
        fixture.write(ACCEPTANCE_CONTRACT_FILE, b"{\"repaired\":true}");
        assert!(!fixture.answers(&record), "{command}: the repair voids it");
        fixture.write("TASK-X-020.md", b"a new task file");
        assert!(!fixture.answers(&record), "{command}: a new task file too");

        std::fs::remove_file(fixture.context.task_root.join("TASK-X-020.md")).unwrap();
        fixture.write(ACCEPTANCE_CONTRACT_FILE, &original.unwrap());
        assert!(
            fixture.answers(&record),
            "{command}: the same content again"
        );
        // The run's own log in the task root is no input.
        fixture.write(".decompose.log", b"progress line");
        assert!(fixture.answers(&record), "{command}: the log is not judged");
    }
}

/// A body refusal judged the frozen body it would replace: a changed body,
/// or a changed frozen chain, voids it; a changed candidate is a new call.
#[test]
fn a_body_refusal_answers_only_while_its_frozen_body_is_unchanged() {
    let fixture = fixture();
    let request = HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap();
    let record = fixture.record(&request);
    assert!(record.result.data[JUDGED_INPUTS].is_string());
    assert!(fixture.answers(&record));

    fixture.write("TASK-X-010.md", b"live-after");
    assert!(!fixture.answers(&record), "the frozen body changed");
    fixture.write("TASK-X-010.md", b"live-before");
    assert!(fixture.answers(&record));

    let other = HostCommandRequest::new("land-task-body", Some(format!("{CANDIDATE}\n"))).unwrap();
    let mut asked = fixture.record(&other);
    asked.call.options.host_command = Some(request.clone());
    assert!(
        !fixture.answers(&asked),
        "an outcome filed under another candidate answers nothing"
    );
}

/// No stamp, no replay: an older binary's record, a pure host read, and a
/// candidate the host could not bind all run again.
#[test]
fn an_outcome_without_a_known_judged_content_never_answers() {
    let fixture = fixture();
    let request = HostCommandRequest::new("freeze-skeleton", Some("candidate".into())).unwrap();
    let mut old = fixture.record(&request);
    old.result
        .data
        .as_object_mut()
        .unwrap()
        .remove(JUDGED_INPUTS);
    assert!(!fixture.answers(&old), "an older binary's record");

    let verify = HostCommandRequest::new("verify-frozen-skeleton", None).unwrap();
    assert_eq!(fixture.executor.judged_inputs(&verify).unwrap(), None);
    assert!(!fixture.answers(&fixture.record(&verify)), "a host read");

    let unbound =
        HostCommandRequest::new("land-task-body", Some("# no frozen subject\n".into())).unwrap();
    assert_eq!(fixture.executor.judged_inputs(&unbound).unwrap(), None);
    assert!(
        !fixture.answers(&fixture.record(&unbound)),
        "an unbound body"
    );

    // A set gate keeps its content manifest identity and is stamped too.
    let lint = HostCommandRequest::new("task-set-lint", None).unwrap();
    let record = fixture.record(&lint);
    assert!(fixture.answers(&record));
    fixture.write("TASK-X-010.md", b"edited");
    assert!(!fixture.answers(&record), "a set gate after an edit");
}

/// Content that changed while the call ran is not known: no stamp.
#[test]
fn a_stamp_needs_the_same_content_before_and_after_the_call() {
    let stamped = |before: Option<&str>, after: Option<&str>| {
        let mut data = serde_json::json!({ "exitCode": 1 });
        stamp(&mut data, before.map(Into::into), after.map(Into::into));
        data.get(JUDGED_INPUTS).cloned()
    };
    assert_eq!(stamped(Some("a"), Some("a")), Some(serde_json::json!("a")));
    assert_eq!(stamped(Some("a"), Some("b")), None);
    assert_eq!(stamped(Some("a"), None), None);
    assert_eq!(stamped(None, Some("a")), None);
    assert_eq!(stamped(None, None), None);
}
