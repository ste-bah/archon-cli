//! A4 through the whole-set freeze: a project outside its repository, no
//! scratch policy, and a check that cannot fail before any implementation.

use super::*;
use crate::command::workflow_task_set::reauthor::AuthorScope;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};

/// Passes on any tree: it reads nothing.
const VACUOUS: &str = "python3 -c 'import sys; sys.exit(0)'";
/// Fails until the implementation writes `out.json`.
const SOUND: &str = "python3 -c 'import json; assert json.load(open(\"out.json\"))[\"valid\"]'";

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A project (tasks, PRD, a draft whose one check is `command`) and, in a
/// separate directory, the repository its `repository.lock` names.
fn outside_set(command: &str) -> (tempfile::TempDir, tempfile::TempDir, PathBuf, PathBuf) {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let repo = outside.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "test"]);
    std::fs::write(repo.join("lib.txt"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    let tasks = project.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let prd = project.path().join("prds/PRD-X.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::write(
        &prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n",
    )
    .unwrap();
    let draft = serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "prds/PRD-X.md", "digest": "pending"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-X-001", "criterion": "untrusted draft summary",
            "check": {"kind": "command", "command": command, "cwd": "project_root"},
            "gap_permitted": false,
            "judgment": {"verdict": "refuted", "counterexample": "untrusted", "reason": "untrusted", "host_call_id": "untrusted"}
        }],
        "supplementary": []
    });
    std::fs::write(
        tasks.join(ACCEPTANCE_CONTRACT_FILE),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    let head = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    archon_workflow::repository_record::write_repository_record(
        &tasks,
        &archon_workflow::repository_record::RepositoryRecordV1 {
            schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
            repository_root: repo.canonicalize().unwrap().display().to_string(),
            base_commit: head.trim().to_string(),
            decomposition_run_id: "fixture".into(),
            recorded_at: "2026-09-01T00:00:00Z".into(),
        },
    )
    .unwrap();
    (project, outside, tasks, prd)
}

#[tokio::test]
async fn a_whole_set_freeze_reauthors_a_check_that_passes_before_any_implementation() {
    let (project, _outside, tasks, prd) = outside_set(VACUOUS);
    let client = Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, SOUND),
        |_, _| true,
    ));
    let scope = AuthorScope::for_task_set(project.path(), &tasks, &prd);
    let prepared = prepare_acceptance_freeze_reauthoring(
        project.path(),
        &tasks,
        &prd,
        GateMode::Enforce,
        client.clone(),
        &scope,
    )
    .await
    .expect("the re-authored check fails before any implementation");
    assert_eq!(client.authored(), 1, "the vacuous check was re-authored");
    let prompt = client.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains(super::super::executability::CANNOT_FAIL),
        "{prompt}"
    );
    assert!(prepared.findings.is_empty(), "{:?}", prepared.findings);
    let contract = prepared.contract().unwrap();
    assert!(matches!(
        &contract.acceptance[0].check,
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } if command == SOUND
    ));
}

/// Enforce mode cannot loop forever on a check no author can make fail: the
/// re-author stops once it makes no progress, nothing is published, and the
/// failure names the PRD owner as the one to resolve it.
#[tokio::test]
async fn an_enforce_freeze_on_a_check_that_cannot_fail_stops_and_escalates() {
    let (project, _outside, tasks, prd) = outside_set(VACUOUS);
    let before = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let client = Arc::new(ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("{VACUOUS} # attempt {attempt}")),
        |_, _| true,
    ));
    let scope = AuthorScope::for_task_set(project.path(), &tasks, &prd);
    let error = prepare_acceptance_freeze_reauthoring(
        project.path(),
        &tasks,
        &prd,
        GateMode::Enforce,
        client.clone(),
        &scope,
    )
    .await
    .expect_err("a check that cannot fail is never published")
    .to_string();
    assert_eq!(
        client.authored(),
        crate::command::workflow_task_set::reauthor::REAUTHOR_ATTEMPTS
    );
    assert!(
        error.contains("escalated to the PRD owner: AC-X-001"),
        "{error}"
    );
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        before,
        "nothing was written"
    );
}

/// Passes once some tracked file says the feature shipped (a word no git
/// template file holds); names no input the mutation could move.
const SHIPPED: &str = "grep -rq feature-shipped-7c1 .";

/// (1) The freeze gate's own probe uses the base the task set recorded, not
/// the repository's HEAD: a check that passes only on a later HEAD (and
/// names no input the mutation could move) is sound on the recorded base.
#[tokio::test]
async fn the_freeze_gate_probes_the_recorded_base_not_head() {
    let (project, outside, tasks, prd) = outside_set(SHIPPED);
    let repo = outside.path().join("repo");
    std::fs::write(repo.join("feature.txt"), "feature-shipped-7c1").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "feature"]);
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    let entry = &mut contract.acceptance[0];
    entry.judgment.verdict = JudgeDecision::Accepted;
    entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: SHIPPED.into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
    };
    let findings = super::super::coverage_gate::pre_implementation_findings(
        project.path(),
        &tasks,
        &prd,
        &contract,
    )
    .await;
    assert!(
        findings.is_empty(),
        "it fails on the recorded base: {:?}",
        findings.iter().map(|f| &f.text).collect::<Vec<_>>()
    );
    // Held to HEAD it would have passed before any implementation.
    let head = super::super::executability::Baseline::head_of(&repo).unwrap();
    let probe = super::super::executability::HostProbe::for_task_set(project.path(), &tasks)
        .with_baseline(head);
    use super::super::executability::ExecutabilityProbe;
    let at_head = probe
        .script_defects(&contract, &["AC-X-001".to_string()].into_iter().collect())
        .await;
    assert!(
        at_head["AC-X-001"].contains(super::super::executability::CANNOT_FAIL),
        "{at_head:?}"
    );
}

/// Fixes 6 and 7 through the freeze: a configured scratch policy that
/// cannot be captured runs nothing, and the freeze refuses operationally --
/// no author is asked to change a check the host could not run.
#[tokio::test]
async fn a_freeze_under_a_broken_scratch_policy_refuses_without_asking_an_author() {
    let (project, _outside, tasks, prd) = outside_set(SOUND);
    std::fs::create_dir_all(project.path().join(".archon")).unwrap();
    std::fs::write(
        project.path().join(".archon/config.toml"),
        "[workflow.acceptance_execution]\nrepository=\"/nonexistent/archon-probe-repo\"\nscratch_parent=\"/nonexistent/scratch\"\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin\"\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
    )
    .unwrap();
    let client = Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, SOUND),
        |_, _| true,
    ));
    let scope = AuthorScope::for_task_set(project.path(), &tasks, &prd);
    let error = prepare_acceptance_freeze_reauthoring(
        project.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        client.clone(),
        &scope,
    )
    .await
    .expect_err("nothing unproven is published")
    .to_string();
    assert!(
        error.contains(super::super::executability::HOST_UNPROVEN),
        "{error}"
    );
    assert_eq!(client.authored(), 0, "no author was asked");
}
