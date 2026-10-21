use super::*;
use crate::command::workflow_task_set::reauthor::cache::{
    PROMPT_AND_JUDGE_SOURCES, prompt_and_judge_source_digest,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn prompt_and_judge_digest_covers_every_static_source_contributor() {
    let listed = PROMPT_AND_JUDGE_SOURCES
        .iter()
        .map(|(name, _)| *name)
        .collect::<std::collections::BTreeSet<_>>();
    let author = include_str!("workflow_acceptance_reauthor_author.rs");
    let reauthor = include_str!("workflow_acceptance_reauthor.rs");
    let live_root = include_str!("workflow_acceptance_live_root.rs");
    let judge = include_str!("workflow_task_set_judge.rs");

    // These references are the non-inline static contributors to the author
    // prompt. Keep their defining sources in the digest closure.
    assert!(author.contains("ENTRY_SHAPES.to_string()"));
    assert!(reauthor.contains("const ENTRY_SHAPES:"));
    assert!(listed.contains("workflow_acceptance_reauthor_author.rs"));
    assert!(listed.contains("workflow_acceptance_reauthor.rs"));
    assert!(author.contains("live_root::check_path_rule("));
    assert!(live_root.contains("pub(crate) fn check_path_rule("));
    assert!(listed.contains("workflow_acceptance_live_root.rs"));

    // The judge instruction is assembled here; its literals and formatting
    // are therefore covered by hashing the source that defines this function.
    assert!(judge.contains("pub(super) fn batched_judge_prompt("));
    assert!(listed.contains("workflow_task_set_judge.rs"));

    let before = prompt_and_judge_source_digest(PROMPT_AND_JUDGE_SOURCES);
    let changed_reauthor = reauthor.replace("ENTRY_SHAPES", "ENTRY_SHAPES_CHANGED");
    let changed_sources = PROMPT_AND_JUDGE_SOURCES
        .iter()
        .map(|(name, source)| {
            if *name == "workflow_acceptance_reauthor.rs" {
                (*name, changed_reauthor.as_bytes())
            } else {
                (*name, *source)
            }
        })
        .collect::<Vec<_>>();
    assert_ne!(
        before,
        prompt_and_judge_source_digest(&changed_sources),
        "an ENTRY_SHAPES-equivalent source change must re-key saved candidates"
    );
}

#[test]
fn reauthor_cache_key_binds_named_file_content() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    std::fs::write(set.project.path().join("input"), "first assertion").unwrap();
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let frozen = contract.acceptance[0].clone();
    let mut item = candidate(&frozen);
    item.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "operation input".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    let probe = DataProbe::default();
    save_with_probe(&set, &contract, &scope, &item, &probe);
    let cache = crate::command::workflow_task_set::reauthor::cache::load(
        &set.tasks,
        &frozen,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
        &probe,
    )
    .unwrap();
    assert!(cache.is_some(), "unchanged named content should reuse");
    std::fs::write(set.project.path().join("input"), "changed assertion").unwrap();
    let cache = crate::command::workflow_task_set::reauthor::cache::load(
        &set.tasks,
        &frozen,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
        &probe,
    )
    .unwrap();
    assert!(cache.is_none(), "changed named content must miss");
}

#[derive(Default)]
struct DataProbe {
    calls: AtomicUsize,
    changed_data: Option<std::path::PathBuf>,
}

#[async_trait::async_trait]
impl crate::command::workflow_task_set::executability::ExecutabilityProbe for DataProbe {
    async fn script_defects(
        &self,
        _contract: &archon_workflow::task_set_contract::AcceptanceContract,
        ids: &std::collections::BTreeSet<String>,
    ) -> std::collections::BTreeMap<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.changed_data.as_ref().is_some_and(|path| path.exists()) {
            ids.iter()
                .map(|id| {
                    (
                        id.clone(),
                        "the changed project data makes this check crash".into(),
                    )
                })
                .collect()
        } else {
            Default::default()
        }
    }
}

fn candidate(
    entry: &archon_workflow::task_set_contract::AcceptanceCriterion,
) -> archon_workflow::task_set_contract::AcceptanceCriterion {
    let mut candidate = entry.clone();
    candidate.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "false".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    candidate.judgment.verdict = archon_workflow::task_set_contract::JudgeDecision::Accepted;
    candidate
}

fn save(
    set: &FrozenSet,
    contract: &archon_workflow::task_set_contract::AcceptanceContract,
    scope: &AuthorScope,
    item: &archon_workflow::task_set_contract::AcceptanceCriterion,
) {
    save_with_probe(set, contract, scope, item, &set.probe);
}

fn save_with_probe(
    set: &FrozenSet,
    contract: &archon_workflow::task_set_contract::AcceptanceContract,
    scope: &AuthorScope,
    item: &archon_workflow::task_set_contract::AcceptanceCriterion,
    probe: &dyn crate::command::workflow_task_set::executability::ExecutabilityProbe,
) {
    crate::command::workflow_task_set::reauthor::cache::store(
        &set.tasks,
        item,
        contract,
        scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
        probe,
    )
    .unwrap();
}

#[tokio::test]
async fn a_matching_saved_candidate_skips_author_and_candidate_judge_calls() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let named = ids(&["AC-F-001"]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let saved = candidate(&contract.acceptance[0]);
    crate::command::workflow_task_set::reauthor::cache::remove(&set.tasks).unwrap();
    save(&set, &contract, &scope, &saved);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "false"), |_, _| true);
    let repaired = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await
    .unwrap();
    assert_eq!(client.authored(), 0);
    assert_eq!(
        *client.judged_ids.lock().unwrap(),
        ["AC-F-001"],
        "only the passability judge runs after the probe"
    );
    assert_eq!(repaired.acceptance[0].check, saved.check);
}

#[tokio::test]
async fn a_reused_candidate_skips_author_and_judge_but_runs_the_probe() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let named = ids(&["AC-F-001"]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let saved = candidate(&contract.acceptance[0]);
    let probe = DataProbe::default();
    save_with_probe(&set, &contract, &scope, &saved, &probe);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "false"), |_, _| true);
    let seeds = std::collections::BTreeMap::new();
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &crate::command::workflow_task_set::reauthor::ReauthorGate {
            probe: &probe,
            seeds: &seeds,
        },
        Some(&set.tasks),
    )
    .await
    .unwrap();
    assert_eq!(client.authored(), 0);
    assert!(client.judged_ids.lock().unwrap().is_empty());
    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        1,
        "the host probe reruns"
    );
}

#[tokio::test]
async fn changed_project_data_can_make_a_reused_candidate_fail_the_proof_gate() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let named = ids(&["AC-F-001"]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let saved = candidate(&contract.acceptance[0]);
    let probe = DataProbe {
        calls: AtomicUsize::new(0),
        changed_data: None,
    };
    save_with_probe(&set, &contract, &scope, &saved, &probe);
    let changed_data = set.project.path().join("probe-data-changed");
    std::fs::write(&changed_data, "changed").unwrap();
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "false"), |_, _| true);
    let probe = DataProbe {
        calls: AtomicUsize::new(0),
        changed_data: Some(changed_data),
    };
    let seeds = std::collections::BTreeMap::new();
    let result = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &crate::command::workflow_task_set::reauthor::ReauthorGate {
            probe: &probe,
            seeds: &seeds,
        },
        Some(&set.tasks),
    )
    .await;
    assert!(
        result.is_err(),
        "the changed-data proof failure cannot be accepted"
    );
    assert!(
        client.authored() > 0,
        "the proof failure returns the check to its author"
    );
    assert!(probe.calls.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn changed_criterion_prompt_instruction_or_judge_ignores_a_saved_candidate() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let named = ids(&["AC-F-001"]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let saved = candidate(&contract.acceptance[0]);
    crate::command::workflow_task_set::reauthor::cache::remove(&set.tasks).unwrap();
    save(&set, &contract, &scope, &saved);

    let mut changed = contract.clone();
    changed.acceptance[0].criterion.push_str(" changed");
    let criterion_client = ScriptedAuthorJudge::new(|_, _| "not json".into(), |_, _| true);
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &criterion_client,
        &changed,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await;
    assert!(criterion_client.authored() > 0);

    // Isolate the legacy-logic case from the earlier changed-criterion call,
    // and write the record with the same probe used for the reuse check.
    let proof = DataProbe::default();
    crate::command::workflow_task_set::reauthor::cache::remove(&set.tasks).unwrap();
    save_with_probe(&set, &contract, &scope, &saved, &proof);
    let path = std::fs::read_dir(set.tasks.join("acceptance-reauthor-candidates"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .next()
        .unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // A record written by the previous format carries the freeze logic stamp.
    // Its old digest remains in both the file name and record, but current key
    // identity is the remaining fields.
    record["key"]["logic_version"] = serde_json::json!(39);
    record["key"]["logic_digest"] = serde_json::json!("old-freeze-logic-digest");
    let old_key_digest = archon_workflow::task_set_contract::content_digest(
        &serde_json::to_vec(&record["key"]).unwrap(),
    );
    record["key_digest"] = serde_json::json!(old_key_digest);
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let old_path = path.parent().unwrap().join(format!(
        "{}.{}.legacy.json",
        archon_workflow::task_set_contract::content_digest(b"AC-F-001"),
        old_key_digest
    ));
    std::fs::rename(&path, old_path).unwrap();
    let logic_client = ScriptedAuthorJudge::new(|_, _| "not json".into(), |_, _| true);
    let seeds = std::collections::BTreeMap::new();
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &logic_client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &crate::command::workflow_task_set::reauthor::ReauthorGate {
            probe: &proof,
            seeds: &seeds,
        },
        Some(&set.tasks),
    )
    .await;
    assert_eq!(
        logic_client.authored(),
        0,
        "logic 39 record is reused under current logic"
    );
    assert_eq!(
        proof.calls.load(Ordering::SeqCst),
        1,
        "proof reruns on reuse"
    );

    crate::command::workflow_task_set::reauthor::cache::remove(&set.tasks).unwrap();
    save(&set, &contract, &scope, &saved);
    let path = std::fs::read_dir(set.tasks.join("acceptance-reauthor-candidates"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .last()
        .unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["key"]["prompt_and_judge_source_digest"] = serde_json::json!("stale-prompt");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let prompt_client = ScriptedAuthorJudge::new(|_, _| "not json".into(), |_, _| true);
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &prompt_client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await;
    assert!(
        prompt_client.authored() > 0,
        "a changed prompt source invalidates reuse"
    );

    crate::command::workflow_task_set::reauthor::cache::remove(&set.tasks).unwrap();
    save(&set, &contract, &scope, &saved);
    let path = std::fs::read_dir(set.tasks.join("acceptance-reauthor-candidates"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .last()
        .unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["key"]["prompt_and_judge_source_digest"] = serde_json::json!("stale-instruction");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let instruction_client = ScriptedAuthorJudge::new(|_, _| "not json".into(), |_, _| true);
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &instruction_client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await;
    assert!(
        instruction_client.authored() > 0,
        "a changed judge instruction invalidates reuse"
    );

    save(&set, &contract, &scope, &saved);
    let judge_client = ScriptedAuthorJudge::new(|_, _| "not json".into(), |_, _| true);
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &judge_client,
        &contract,
        &named,
        &scope,
        "different-model",
        &set.gate(),
        Some(&set.tasks),
    )
    .await;
    assert!(judge_client.authored() > 0);
}

#[tokio::test]
async fn changed_candidate_check_text_discards_a_saved_candidate() {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let named = ids(&["AC-F-001"]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    save(&set, &contract, &scope, &candidate(&contract.acceptance[0]));

    let path = std::fs::read_dir(set.tasks.join("acceptance-reauthor-candidates"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .next()
        .unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["key"]["check"]["command"] = serde_json::json!("echo changed");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();

    let client = ScriptedAuthorJudge::new(|_, _| "not json".into(), |_, _| true);
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await;
    assert!(
        client.authored() > 0,
        "changed check text invalidates reuse"
    );
}

#[tokio::test]
async fn saved_candidates_that_fail_proof_return_to_authoring() {
    let set = frozen_set(&[
        ("AC-F-001", "false", false),
        ("AC-F-002", "false", false),
        ("AC-F-003", "false", false),
        ("AC-F-004", "false", false),
        ("AC-F-005", "false", false),
    ]);
    let named = ids(&["AC-F-001", "AC-F-002", "AC-F-003", "AC-F-004", "AC-F-005"]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    for entry in contract.acceptance.iter().take(3) {
        save(&set, &contract, &scope, &candidate(entry));
    }
    let client = ScriptedAuthorJudge::new(|_, _| "not json".into(), |_, _| true);
    let _ = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await;
    assert_eq!(client.authored(), 8);
    let prompts = client.prompts.lock().unwrap();
    let authored_ids: BTreeSet<String> = prompts
        .iter()
        .map(|prompt| {
            let line = prompt
                .lines()
                .find_map(|line| line.strip_prefix("The entry being replaced: "))
                .unwrap();
            serde_json::from_str::<serde_json::Value>(line).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(authored_ids, ids(&["AC-F-004", "AC-F-005"]));
}
