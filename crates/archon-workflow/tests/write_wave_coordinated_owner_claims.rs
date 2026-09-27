//! Issue-121 follow-up: coordinated write mode claims other tasks' files the
//! way worktree mode does.
//!
//! Issue-121 appended an owner claim per task outside the branch's own to
//! the worktree wave's claims only; a coordinated wave's adapter still saw
//! the sibling items alone, so a single-item coordinated round was granted
//! another task's declared file. This drives the production coordinated
//! fanout and checks the claim list its branch is dispatched with, and that
//! the adapter refuses the other task's file from it, as worktree mode does.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::sync::Mutex;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::call_data::v2_agent_request;
use archon_workflow::v2::write::run_write_capable_v2_fanout;
use archon_workflow::*;
use serde_json::{Value, json};
use support::{Fixture, git};

const OWN: &str = "crates/a/src/lib.rs";
const OTHER: &str = "crates/a/src/other.rs";

/// Records the claim list and asks the real adapter to judge a result that
/// changes `reports`.
struct Recording {
    reports: Vec<&'static str>,
    claims: Mutex<Vec<Value>>,
    verdicts: Mutex<Vec<Result<(), String>>>,
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for Recording {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        task: &str,
        root: Option<String>,
        execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter,
        _store: Option<&WorkflowV2ResultStore>,
        universe: Option<&task_universe::WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let claims = execution.call.options.extra.get("wave_claims").cloned();
        self.claims
            .lock()
            .unwrap()
            .push(claims.unwrap_or(Value::Null));
        let request = v2_agent_request(task, root, execution, universe);
        let mut result = WorkflowV2Result::accepted("implemented");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "changed the file",
        ));
        result.files_changed = self
            .reports
            .iter()
            .map(|path| WorkflowV2FileRecord::new(*path))
            .collect();
        let raw = serde_json::to_string(&result).unwrap();
        self.verdicts.lock().unwrap().push(
            adapter
                .parse_agent_output(&request, &raw)
                .map(|_| ())
                .map_err(|err| err.to_string()),
        );
        Ok(WorkflowV2Result::noop("judged"))
    }
}

fn world() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [(OWN, "// a\n"), (OTHER, "// other\n")] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crate"]);
    let task = |id: &str, files: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        files_expected_to_change: files.iter().map(|f| (*f).to_string()).collect(),
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![task("TASK-001", &[OWN]), task("TASK-002", &[OTHER])],
    });
    f
}

async fn coordinated(f: &Fixture, reports: Vec<&'static str>) -> Recording {
    let call = WorkflowV2HostCall {
        id: "owner".into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".into()),
            task: Some("Implement the item now.".into()),
            target_files_from_item: true,
            ..Default::default()
        },
    };
    let mut branch = call.clone();
    branch.id = "owner-0".into();
    branch.method = WorkflowV2HostMethod::Implementation;
    branch.options.target_files = vec![OWN.into()];
    let item = WorkflowV2FanoutItem::read_only(
        "owner-0",
        "coder",
        branch,
        json!({"item": {"item_id": "owner-0", "canonical_task_ids": ["TASK-001"],
            "target_files": [OWN], "work_type": "implementation"}}),
    );
    let dispatch = Recording {
        reports,
        claims: Mutex::new(Vec::new()),
        verdicts: Mutex::new(Vec::new()),
    };
    let _ = run_write_capable_v2_fanout(
        "fallback objective",
        Some(f.repo.to_str().unwrap()),
        WorkflowV2CallExecution {
            call,
            input: json!({}),
            depends_on: vec![],
        },
        WorkflowV2AgentAdapter::new(),
        &dispatch,
        &f.v2,
        &f.store,
        &f.run,
        true,
        vec![item],
        f.universe.as_ref(),
        None,
    )
    .await;
    dispatch
}

#[tokio::test]
async fn a_coordinated_branch_is_dispatched_the_other_tasks_claims_and_refused_their_file() {
    let f = world();
    let refused = coordinated(&f, vec![OWN, OTHER]).await;
    let claims = refused.claims.lock().unwrap().clone();
    assert_eq!(claims.len(), 1, "{claims:?}");
    assert!(
        claims[0]
            .as_array()
            .unwrap()
            .contains(&json!({"item_id": "task-owner:TASK-002", "owned": [OTHER]})),
        "{claims:?}"
    );
    let verdicts = refused.verdicts.lock().unwrap().clone();
    let err = verdicts[0]
        .clone()
        .expect_err("another task's file is refused");
    assert!(err.contains(OTHER), "{err}");
    // Its own file alone is judged as before.
    let own = coordinated(&f, vec![OWN]).await;
    assert_eq!(own.verdicts.lock().unwrap()[0], Ok(()));
}
