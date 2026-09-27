//! Per-check repair: only named entries change, the chain republishes whole.

use super::test_fixture::{FrozenSet, assert_only_named_entries_changed, frozen_set};
use super::*;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};

fn request<'a>(set: &'a FrozenSet, ids: &'a BTreeSet<String>) -> ReauthorRequest<'a> {
    ReauthorRequest {
        project_root: set.project.path(),
        tasks_root: &set.tasks,
        prd_path: &set.prd,
        mode: GateMode::Observe,
        ids,
    }
}

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| id.to_string()).collect()
}

#[tokio::test]
async fn reauthor_republishes_contract_lock_skeleton_and_pin_as_one_verified_chain() {
    let set = frozen_set(&[
        ("AC-F-001", "jq -e '.a == true' out.json", true),
        ("AC-F-002", "jq -e '.b == true' out.json", false),
        ("AC-F-003", "jq -e '.c == true' out.json", true),
    ]);
    let before = set.contract_bytes();
    let old_pin = set.pin();
    let named = ids(&["AC-F-002"]);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "jq -e '.b == true and .d == 1' out.json"),
        |_, _| true,
    );
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    let result = reauthor_and_republish(&client, request(&set, &named), &scope)
        .await
        .expect("repair publishes");
    assert_only_named_entries_changed(&before, &set.contract_bytes(), &named);
    let pin = set.pin();
    assert_ne!(pin.acceptance_digest, old_pin.acceptance_digest);
    assert_eq!(pin.freeze_event_id, result.freeze_event_id);
    assert_eq!(pin.skeleton_digest, result.skeleton_digest);
    assert!(
        result.skeleton_digest.is_some(),
        "the skeleton is re-chained in the same step"
    );
    validate_full_chain(&set.tasks, &pin).expect("the whole chain verifies with no follow-up");
    let skeleton: TaskSkeleton =
        serde_json::from_slice(&std::fs::read(set.tasks.join(TASK_SKELETON_FILE)).unwrap())
            .unwrap();
    assert_eq!(skeleton.acceptance_digest, pin.acceptance_digest);
    assert!(non_accepted_ids(&set.contract()).is_empty());
}

#[tokio::test]
async fn reauthor_refuses_an_unknown_check_and_writes_nothing() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let before = set.chain_bytes();
    let named = ids(&["AC-F-404"]);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "true"), |_, _| true);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    let error = reauthor_and_republish(&client, request(&set, &named), &scope)
        .await
        .expect_err("unknown id")
        .to_string();
    assert!(error.contains("not in the frozen contract"), "{error}");
    assert_eq!(set.chain_bytes(), before);
    assert_eq!(client.authored(), 0);
}

#[tokio::test]
async fn reauthor_refuses_to_leave_an_unnamed_refuted_check_published() {
    let set = frozen_set(&[
        ("AC-F-001", "jq -e '.a == true' out.json", false),
        ("AC-F-002", "jq -e '.b == true' out.json", false),
    ]);
    let before = set.chain_bytes();
    let named = ids(&["AC-F-001"]);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "true"), |_, _| true);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    let error = reauthor_and_republish(&client, request(&set, &named), &scope)
        .await
        .expect_err("AC-F-002 would stay refuted")
        .to_string();
    assert!(error.contains("AC-F-002"), "{error}");
    assert_eq!(set.chain_bytes(), before);
}

#[tokio::test]
async fn a_repair_the_judge_never_accepts_fails_bounded_and_writes_nothing() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let before = set.chain_bytes();
    let named = ids(&["AC-F-001"]);
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("jq -e '.a{attempt}' out.json")),
        |_, _| false,
    );
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    let error = reauthor_and_republish(&client, request(&set, &named), &scope)
        .await
        .expect_err("still refuted")
        .to_string();
    assert!(error.contains("after 3 re-author attempt(s)"), "{error}");
    assert_eq!(set.chain_bytes(), before);
    assert_eq!(client.authored(), reauthor::REAUTHOR_ATTEMPTS);
}

#[test]
fn launch_is_refused_while_the_bound_contract_carries_a_refuted_check() {
    let set = frozen_set(&[
        ("AC-F-001", "jq -e '.a == true' out.json", true),
        ("AC-F-002", "jq -e '.b == true' out.json", false),
    ]);
    let error = refuse_unaccepted_launch(set.project.path(), &set.tasks)
        .expect_err("refuted check bound")
        .to_string();
    assert!(error.contains("refusing to launch"), "{error}");
    assert!(
        error.contains(&format!(
            "archon workflow freeze-acceptance --reauthor AC-F-002 --tasks {}",
            set.tasks.display()
        )),
        "{error}"
    );
    let clean = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", true)]);
    refuse_unaccepted_launch(clean.project.path(), &clean.tasks)
        .expect("accepted contract launches");
}

/// Dry run of `--reauthor` against a COPY of a real frozen task directory,
/// with a scripted author and judge. Skipped unless
/// `ARCHON_REAUTHOR_DRY_RUN=<project>|<tasks dir>|<prd>|<check id>` names the
/// copy; the ids come from the environment so no fixture id lives in code.
#[tokio::test]
async fn reauthor_dry_run_against_a_copied_task_directory() {
    let Ok(spec) = std::env::var("ARCHON_REAUTHOR_DRY_RUN") else {
        return;
    };
    let parts = spec.split('|').collect::<Vec<_>>();
    let [project, tasks, prd, check] = parts.as_slice() else {
        panic!("ARCHON_REAUTHOR_DRY_RUN must be <project>|<tasks>|<prd>|<check id>");
    };
    let (project, tasks, prd) = (Path::new(project), Path::new(tasks), Path::new(prd));
    let before = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let named = ids(&[check]);
    // A generic rewrite: the same command with one more assertion line.
    let client = ScriptedAuthorJudge::new(
        |entry, _| {
            let command = entry["check"]["command"].as_str().unwrap_or("true");
            command_entry(entry, &format!("{command}\n# dry-run re-author"))
        },
        |_, _| true,
    );
    let scope = AuthorScope::for_task_set(project, tasks, prd);
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: project,
            tasks_root: tasks,
            prd_path: prd,
            mode: GateMode::Observe,
            ids: &named,
        },
        &scope,
    )
    .await
    .expect("dry-run repair publishes");
    let after = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    assert_only_named_entries_changed(&before, &after, &named);
    let pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(project, tasks)).unwrap())
            .unwrap();
    validate_full_chain(tasks, &pin).expect("skeleton chain verifies");
    eprintln!(
        "dry run: {check} re-authored; event={} acceptance={} skeleton={:?}; other entries byte-identical; full chain verifies",
        result.freeze_event_id, result.acceptance_digest, result.skeleton_digest
    );
}
