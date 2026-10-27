use super::*;

#[test]
fn an_unstamped_entry_cache_record_is_a_miss() {
    let temp = tempfile::tempdir().unwrap();
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    let entry = cache_contract().acceptance[0].clone();
    let named_digest = super::super::judge::named_files_digest(
        &entry,
        &super::super::judge::JudgeRoots::default(),
    );
    let key = "seeded-entry";
    let cache = temp
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("judge-entry");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join(format!("{key}.json")),
        serde_json::json!({ "key": key, "judgment": entry.judgment }).to_string(),
    )
    .unwrap();

    assert!(
        super::super::incremental::load_entry(
            temp.path(),
            key,
            &expected_logic_identity(&client),
            &named_digest,
        )
        .is_none(),
        "an unstamped verdict must miss the entry cache"
    );
}

#[test]
fn stale_entry_cache_identity_and_named_digest_are_misses() {
    let temp = tempfile::tempdir().unwrap();
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    let mut entry = cache_contract().acceptance[0].clone();
    let roots = super::super::judge::JudgeRoots::default();
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        &mut entry,
        "model-x",
        client.provider_id().as_deref(),
        &roots,
    );
    let named_digest = super::super::judge::named_files_digest(&entry, &roots);
    let key = "seeded-entry";
    let cache = temp
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("judge-entry");
    std::fs::create_dir_all(&cache).unwrap();

    for stale_field in ["logic_identity", "named_file_digest"] {
        let mut record = serde_json::json!({ "key": key, "judgment": entry.judgment });
        record["judgment"]["sampling"][stale_field] = serde_json::json!("stale-stamp");
        std::fs::write(cache.join(format!("{key}.json")), record.to_string()).unwrap();
        assert!(
            super::super::incremental::load_entry(
                temp.path(),
                key,
                &expected_logic_identity(&client),
                &named_digest,
            )
            .is_none(),
            "a stale {stale_field} must miss the entry cache"
        );
    }
}

#[test]
fn a_later_stamp_does_not_overwrite_the_digest_of_an_existing_judgment() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    std::fs::write(&input, "judged content").unwrap();
    let roots = super::super::judge::JudgeRoots {
        project: Some(temp.path().to_path_buf()),
        repository: None,
        task_root: Some(temp.path().to_path_buf()),
        repository_commit: None,
    };
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    let mut entry = cache_contract().acceptance[0].clone();
    entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "operation input".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        &mut entry,
        "model-x",
        client.provider_id().as_deref(),
        &roots,
    );
    let original_stamp = entry.judgment.sampling.as_ref().unwrap()["named_file_digest"].clone();

    std::fs::write(input, "changed content").unwrap();
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        &mut entry,
        &client.resolve_model_alias("sonnet"),
        client.provider_id().as_deref(),
        &roots,
    );

    assert_eq!(
        entry.judgment.sampling.as_ref().unwrap()["named_file_digest"],
        original_stamp,
        "a later stamp must not make an older judgment look current"
    );
}
