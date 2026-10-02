//! PLAN-11, through the production write wave: the hold runs only for a
//! branch about to land, after its drops (a refused branch keeps its work
//! and records nothing), and a project-rooted check script seeded into the
//! worktree as a project input is held at landing like a repository source.
#[path = "support/write_wave_fixture.rs"]
mod support;

#[path = "support/check_source_world.rs"]
mod world;

use std::path::Path;

use archon_workflow::acceptance_scratch::ScratchPolicy;
use archon_workflow::check_source_requests::{self as requests};
use archon_workflow::check_source_resolve::SourceRoot;
use archon_workflow::task_set_contract::{ACCEPTANCE_CONTRACT_FILE, AcceptanceContract};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};
use world::*;

/// Item 2: a branch refused for a forbidden path is never held -- no
/// request is recorded, and its worktree work (the pinned-test edit
/// included) is left for its next attempt rather than restored away.
#[tokio::test]
async fn a_refused_branch_holds_nothing_and_records_no_request() {
    let mut f = Fixture::new();
    frozen_task_set(&f);
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-001".into(),
            source_path: "tasks/TASK-001.md".into(),
            files_forbidden_to_change: vec!["other.txt".into()],
            ..Default::default()
        }],
    });
    f.wave(
        "refused",
        vec![(
            vec!["tests/judge.rs"],
            Edits {
                files: vec![
                    ("tests/judge.rs", "#[test]\nfn judges() {}\n"),
                    ("other.txt", "forbidden edit\n"),
                ],
                report: vec!["tests/judge.rs", "other.txt"],
                via_adapter: true,
            },
        )],
    )
    .await;
    let result = f.branch_result("refused", "refused-0");
    assert_eq!(
        result.data["forbidden_paths_changed"],
        json!(["other.txt"]),
        "{result:#?}"
    );
    assert!(
        result.data.get("check_source_held").is_none(),
        "{result:#?}"
    );
    assert!(
        requests::all(&run_root(&f)).unwrap().is_empty(),
        "nothing recorded"
    );
    assert_eq!(
        git(&f.repo, &["show", "HEAD:tests/judge.rs"]),
        JUDGE_TEST.trim_end()
    );
}

const CHECK: &str = ".archon/lab/checks/check.sh";

fn policy(f: &Fixture, scratch: &Path) -> ScratchPolicy {
    let project = project_root(f)
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    ScratchPolicy {
        repository: f
            .repo
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: scratch.to_path_buf(),
        project_inputs: vec![".archon/lab".into()],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: std::env::join_paths([archon_shell::resolve_posix_shell()
            .parent()
            .unwrap()])
        .unwrap()
        .into_string()
        .unwrap(),
        environment: Default::default(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 60,
        output_bytes: 4096,
        scratch_bytes: 1 << 30,
        build_cache: None,
    }
}

/// Item 5: a project-input copy of a pinned project-rooted check script,
/// changed in the worktree, is held at landing: the project keeps its
/// pinned script, a request is recorded against the project root, and the
/// branch's repository work lands.
#[tokio::test]
async fn a_project_input_copy_of_a_pinned_script_is_held_at_landing() {
    let temp = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    git(&f.repo, &["add", ".gitignore"]);
    git(&f.repo, &["commit", "-qm", "ignore project state"]);
    let project = project_root(&f);
    let script = project.join(CHECK);
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, "test -f built || exit 1\n").unwrap();
    let tasks = project.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let contract: AcceptanceContract = serde_json::from_value(json!({
        "schema_version": 1, "prd": {"path": "prd.md", "digest": "d"}, "gap_policy": {},
        "acceptance": [{"id": "AC-1", "criterion": "built", "judgment": {"verdict": "accepted",
            "counterexample": "none", "reason": "ok", "host_call_id": "judge"},
            "check": {"kind": "command", "command": format!("bash {CHECK}"), "cwd": "project_root"}}]
    }))
    .unwrap();
    std::fs::write(
        tasks.join(ACCEPTANCE_CONTRACT_FILE),
        serde_json::to_vec_pretty(&contract).unwrap(),
    )
    .unwrap();
    let metadata = json!({"observer_snapshot": {"native_execution": {
        "policy": policy(&f, &temp.path().join("scratch")),
        "source_commit": git(&f.repo, &["rev-parse", "HEAD"])}}});
    let path = run_root(&f).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let out = f
        .wave(
            "input",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        (
                            "",
                            "\u{0}run:printf 'exit 0\\n' > .archon/lab/checks/check.sh",
                        ),
                    ],
                    report: vec!["owned.txt"],
                    via_adapter: false,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(
        std::fs::read_to_string(&script).unwrap(),
        "test -f built || exit 1\n",
        "the project keeps its pinned script"
    );
    let pending = requests::pending(&run_root(&f)).unwrap();
    assert_eq!(pending.len(), 1, "{pending:#?}");
    assert_eq!(
        (pending[0].root, pending[0].path.as_str()),
        (SourceRoot::Project, CHECK)
    );
    let proposed = requests::blobs(&run_root(&f))
        .get(pending[0].proposed_digest.as_ref().unwrap())
        .unwrap();
    assert_eq!(proposed, b"exit 0\n");
}
