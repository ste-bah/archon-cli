//! Issue 337: what the gate of an unpublished outcome reads, for the real
//! executor. The host-taken pause records this digest, and a resume replays
//! the outcome only while the digest now is the one at the pause
//! (`workflow_live_v2_script_judged_resume_tests` drives that end to end).

use std::sync::Arc;

use archon_workflow::{HostCommandRequest, WorkflowStore, WorkflowV2CallRecord};

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, fixed_decomposition_catalog,
};
use super::workflow_host_command_exec::{FixedHostCommandExecutor, WorkflowHostCommandExecutor};
use super::workflow_host_command_exec_tests::{PreparedBodyProcess, context, seed_frozen_chain};
use super::workflow_host_command_judged_inputs::identity_holds;
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
            overflow_stdout: false,
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
    /// The unpublished outcome of `request`, filed under its identity.
    fn record(&self, request: &HostCommandRequest) -> WorkflowV2CallRecord {
        let mut options = archon_workflow::WorkflowV2HostOptions::default();
        options.host_command = Some(request.clone());
        let mut result = archon_workflow::WorkflowV2Result::default();
        result.data = serde_json::json!({ "exitCode": 1 });
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

    /// What the gate of `request` reads now, as a pause records it.
    fn judged(&self, request: &HostCommandRequest) -> Option<String> {
        self.executor.judged_inputs(request).unwrap()
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.context.task_root.join(name), bytes).unwrap();
    }
}

/// The reviewer's case: a freeze refusal that the task root caused; the
/// operator repairs the task root after the pause. The digest the pause
/// recorded no longer matches; the same content again matches again. The
/// run's own log in the task root is no input.
#[test]
fn a_freeze_gate_reads_the_task_root_the_pause_recorded() {
    use archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    for command in ["freeze-acceptance", "freeze-skeleton"] {
        let fixture = fixture();
        let request = HostCommandRequest::new(command, Some("candidate".into())).unwrap();
        let at_pause = fixture.judged(&request);
        assert!(at_pause.is_some(), "{command}");
        assert_eq!(fixture.judged(&request), at_pause, "{command}: unchanged");

        let original = std::fs::read(fixture.context.task_root.join(ACCEPTANCE_CONTRACT_FILE));
        fixture.write(ACCEPTANCE_CONTRACT_FILE, b"{\"repaired\":true}");
        assert_ne!(fixture.judged(&request), at_pause, "{command}: a repair");
        fixture.write(ACCEPTANCE_CONTRACT_FILE, &original.unwrap());
        fixture.write("TASK-X-020.md", b"a new task file");
        assert_ne!(fixture.judged(&request), at_pause, "{command}: a new task");

        std::fs::remove_file(fixture.context.task_root.join("TASK-X-020.md")).unwrap();
        assert_eq!(fixture.judged(&request), at_pause, "{command}: restored");
        fixture.write(".decompose.log", b"progress line");
        assert_eq!(fixture.judged(&request), at_pause, "{command}: the log");
    }
}

/// A body gate reads the frozen body it would replace; a changed candidate
/// is a new call, so an outcome filed under another candidate answers
/// nothing by identity.
#[test]
fn a_body_gate_reads_its_frozen_body_and_answers_only_its_own_candidate() {
    let fixture = fixture();
    let request = HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap();
    let at_pause = fixture.judged(&request);
    assert!(at_pause.is_some());

    fixture.write("TASK-X-010.md", b"live-after");
    assert_ne!(
        fixture.judged(&request),
        at_pause,
        "the frozen body changed"
    );
    fixture.write("TASK-X-010.md", b"live-before");
    assert_eq!(fixture.judged(&request), at_pause);

    let record = fixture.record(&request);
    assert!(identity_holds(&fixture.executor, &record).unwrap());
    let other = HostCommandRequest::new("land-task-body", Some(format!("{CANDIDATE}\n"))).unwrap();
    let mut asked = fixture.record(&other);
    asked.call.options.host_command = Some(request.clone());
    assert!(
        !identity_holds(&fixture.executor, &asked).unwrap(),
        "an outcome filed under another candidate answers nothing"
    );
}

/// No digest, no replay: a pure host read and a candidate the host could
/// not bind have none. A set gate has one, over the whole task set.
#[test]
fn only_a_gate_that_reads_the_task_root_has_a_digest() {
    let fixture = fixture();
    let verify = HostCommandRequest::new("verify-frozen-skeleton", None).unwrap();
    assert_eq!(fixture.judged(&verify), None, "a host read");
    let unbound =
        HostCommandRequest::new("land-task-body", Some("# no frozen subject\n".into())).unwrap();
    assert_eq!(fixture.judged(&unbound), None, "an unbound body");

    let lint = HostCommandRequest::new("task-set-lint", None).unwrap();
    let at_pause = fixture.judged(&lint);
    assert!(at_pause.is_some());
    fixture.write("TASK-X-010.md", b"edited");
    assert_ne!(fixture.judged(&lint), at_pause, "a set gate after an edit");
}
