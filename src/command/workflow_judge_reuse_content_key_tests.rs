use super::*;
use archon_workflow::task_set_contract::AcceptanceCriterion;

fn roots(temp: &tempfile::TempDir, tasks: &Path) -> super::super::judge::JudgeRoots {
    super::super::judge::JudgeRoots {
        project: Some(temp.path().to_path_buf()),
        repository: None,
        task_root: Some(tasks.to_path_buf()),
        repository_commit: None,
    }
}

fn name_input(entry: &mut AcceptanceCriterion, path: &str) {
    entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: format!("operation {path}"),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
}

async fn run_judge(
    temp: &tempfile::TempDir,
    tasks: &Path,
    contract: AcceptanceContract,
    client: &RecordingJudge,
    roots: &super::super::judge::JudgeRoots,
) -> AcceptanceContract {
    let expected = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect();
    super::super::incremental::judge(temp.path(), tasks, client, contract, &expected, None, roots)
        .await
        .unwrap()
}

async fn seeded_named_entry() -> (
    tempfile::TempDir,
    PathBuf,
    AcceptanceContract,
    RecordingJudge,
    super::super::judge::JudgeRoots,
) {
    let (temp, tasks, _prd, mut contract, _initial_client) = frozen().await;
    let input_size = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES / 2;
    std::fs::write(temp.path().join("input"), "i".repeat(input_size)).unwrap();
    let roots = roots(&temp, &tasks);
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    name_input(&mut contract.acceptance[0], "input");
    contract.acceptance[0].judgment.sampling = Some(serde_json::json!({
        "model": client.resolve_model_alias("sonnet"),
        "provider": client.provider_id(),
    }));
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        &mut contract.acceptance[0],
        &client.resolve_model_alias("sonnet"),
        client.provider_id().as_deref(),
        &roots,
    );
    let ids = BTreeSet::from([contract.acceptance[0].id.clone()]);
    super::super::incremental::save_judged_entries(
        temp.path(),
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &client,
        &contract,
        &ids,
        &BTreeSet::new(),
        &roots,
    )
    .unwrap();
    (temp, tasks, contract, client, roots)
}

#[tokio::test]
async fn inserting_and_reordering_entries_keeps_unchanged_verdicts_reused() {
    let (temp, tasks, mut contract, client, roots) = seeded_named_entry().await;
    let large_size = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES * 3 / 4;
    std::fs::write(temp.path().join("large-input"), "l".repeat(large_size)).unwrap();
    let mut inserted = contract.acceptance[0].clone();
    inserted.id = "AC-X-003".into();
    name_input(&mut inserted, "large-input");
    inserted.judgment.verdict = JudgeDecision::Refuted;
    contract.acceptance.insert(0, inserted);

    let first = run_judge(&temp, &tasks, contract.clone(), &client, &roots).await;
    assert_eq!(
        *client.batches.lock().unwrap(),
        vec![vec!["AC-X-003".to_string()]],
        "the inserted entry is judged while the unchanged accepted entry is reused"
    );
    assert_eq!(first.acceptance[1].id, "AC-X-001");

    client.batches.lock().unwrap().clear();
    contract.acceptance.swap(0, 1);
    run_judge(&temp, &tasks, contract, &client, &roots).await;
    assert!(
        client.batches.lock().unwrap().is_empty(),
        "reordering must keep both saved verdicts reusable"
    );
}

#[tokio::test]
/// #549 replaces `changed_and_deleted_named_inputs_miss_the_saved_verdict`:
/// the changed input here is above the judged-source limit of one check, so
/// the missed verdict is now recorded as not judged instead of asking the
/// judge; every other assertion is kept.
async fn changed_and_deleted_named_inputs_miss_the_saved_verdict_549() {
    let (temp, tasks, mut contract, client, roots) = seeded_named_entry().await;
    let oversized = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES + 1;
    std::fs::write(temp.path().join("input"), "a".repeat(oversized)).unwrap();
    contract.acceptance[0].judgment.sampling = Some(serde_json::json!({
        "model": client.resolve_model_alias("sonnet"),
        "provider": client.provider_id(),
    }));
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        &mut contract.acceptance[0],
        &client.resolve_model_alias("sonnet"),
        client.provider_id().as_deref(),
        &roots,
    );
    super::super::incremental::save_judged_entries(
        temp.path(),
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &client,
        &contract,
        &BTreeSet::from([contract.acceptance[0].id.clone()]),
        &BTreeSet::new(),
        &roots,
    )
    .unwrap();
    let old_key = super::super::incremental::entry_key(
        &client,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[0],
        &roots,
    )
    .unwrap();
    std::fs::write(temp.path().join("input"), "c".repeat(oversized)).unwrap();
    let changed_key = super::super::incremental::entry_key(
        &client,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[0],
        &roots,
    )
    .unwrap();
    assert_ne!(old_key, changed_key, "changed content must change the key");
    let changed = run_judge(&temp, &tasks, contract.clone(), &client, &roots).await;
    assert!(
        client.batches.lock().unwrap().is_empty()
            && super::super::judge::not_judged(&changed.acceptance[0]).is_some(),
        "changed content must not reuse the saved verdict; above the limit it is not judged"
    );

    client.batches.lock().unwrap().clear();
    std::fs::remove_file(temp.path().join("input")).unwrap();
    let absent_key = super::super::incremental::entry_key(
        &client,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[0],
        &roots,
    )
    .unwrap();
    assert_ne!(changed_key, absent_key, "deletion must change the key");
    run_judge(&temp, &tasks, contract, &client, &roots).await;
    assert_eq!(
        *client.batches.lock().unwrap(),
        vec![vec!["AC-X-001".to_string()]],
        "a deleted named input must be judged again"
    );
}

#[tokio::test]
/// #549 replaces `oversized_named_input_key_is_independent_of_other_entries`:
/// an entry whose own named file exceeds the judged-source limit of one
/// check is no longer sent to the judge (it was refuted for content it was
/// not shown); every key assertion is kept.
async fn oversized_named_input_key_is_independent_and_the_entry_is_not_judged_549() {
    let (temp, tasks, mut contract, client, roots) = seeded_named_entry().await;
    let size = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES + 1;
    std::fs::write(temp.path().join("oversized-a"), "o".repeat(size)).unwrap();
    std::fs::write(temp.path().join("oversized-b"), "b".repeat(size)).unwrap();
    let mut earlier = contract.acceptance[0].clone();
    earlier.id = "AC-X-003".into();
    name_input(&mut earlier, "oversized-a");
    earlier.judgment.verdict = JudgeDecision::Refuted;
    contract.acceptance.insert(0, earlier);
    let oversized_key = super::super::incremental::entry_key(
        &client,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[0],
        &roots,
    );
    let target_key = super::super::incremental::entry_key(
        &client,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[1],
        &roots,
    );
    let first = run_judge(&temp, &tasks, contract.clone(), &client, &roots).await;
    assert!(
        client.batches.lock().unwrap().is_empty(),
        "an entry whose own named file exceeds the limit is not judged"
    );
    assert!(super::super::judge::not_judged(&first.acceptance[0]).is_some());

    client.batches.lock().unwrap().clear();
    name_input(&mut contract.acceptance[0], "oversized-b");
    let next_round_key = super::super::incremental::entry_key(
        &client,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[1],
        &roots,
    );
    assert_eq!(
        target_key, next_round_key,
        "a different oversized earlier input cannot change this key"
    );
    run_judge(&temp, &tasks, contract.clone(), &client, &roots).await;
    assert!(
        client.batches.lock().unwrap().is_empty(),
        "the unchanged entry stays reused across rounds with different earlier inputs"
    );
    std::fs::write(temp.path().join("oversized-a"), "z".repeat(size)).unwrap();
    let changed_earlier_key = super::super::incremental::entry_key(
        &client,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[0],
        &roots,
    );
    assert_ne!(
        oversized_key, changed_earlier_key,
        "full content remains part of the key even above the prompt budget"
    );
}
