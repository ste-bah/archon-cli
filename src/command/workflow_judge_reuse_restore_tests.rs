use super::*;

#[test]
fn workflow_judge_reuse_pinned_verdict_is_not_restored_after_check_or_covers_changes() {
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    for field in ["check", "covers"] {
        let mut current = cache_contract();
        let mut pinned = current.clone();
        let old = &mut pinned.acceptance[0];
        old.judgment.verdict = JudgeDecision::Accepted;
        old.judgment.sampling = Some(serde_json::json!({
            "model": client.resolve_model_alias("sonnet"),
            "provider": client.provider_id(),
            "logic_identity": super::expected_logic_identity(&client)
        }));
        crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
            old,
            &client.resolve_model_alias("sonnet"),
            client.provider_id().as_deref(),
            &super::super::judge::JudgeRoots::default(),
        );
        match field {
            "check" => {
                current.acceptance[0].check = serde_json::from_value(
                    serde_json::json!({"kind":"command","command":"false","cwd":"project_root"}),
                )
                .unwrap();
            }
            "covers" => current.acceptance[0].covers.push("REQ-CHANGED".into()),
            _ => unreachable!(),
        }
        current.acceptance[0].judgment.verdict = JudgeDecision::Refuted;
        let restored = super::super::incremental::keep_pinned_entries(
            &mut current,
            &pinned,
            &client,
            &super::super::judge::JudgeRoots::default(),
        );
        println!(
            "EVIDENCE #419 changed-{field} BEFORE: fresh={:?}, pinned-check={:?}, pinned-covers={:?}",
            current.acceptance[0].judgment.verdict,
            pinned.acceptance[0].check,
            pinned.acceptance[0].covers
        );
        println!(
            "EVIDENCE #419 changed-{field} AFTER: restored={}, verdict={:?}",
            restored.contains("AC-1"),
            current.acceptance[0].judgment.verdict
        );
        assert!(
            !restored.contains("AC-1"),
            "changed {field} must retain the fresh refusal"
        );
        assert_eq!(
            current.acceptance[0].judgment.verdict,
            JudgeDecision::Refuted
        );
    }
}

#[test]
fn workflow_judge_reuse_identical_pinned_fields_restore_but_each_changed_field_keeps_fresh_refusal()
{
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    for field in ["unchanged", "check", "covers"] {
        let mut current = cache_contract();
        let mut pinned = current.clone();
        let old = &mut pinned.acceptance[0];
        old.judgment.verdict = JudgeDecision::Accepted;
        old.judgment.sampling = Some(serde_json::json!({
            "model": client.resolve_model_alias("sonnet"),
            "provider": client.provider_id(),
            "logic_identity": super::expected_logic_identity(&client)
        }));
        crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
            old,
            &client.resolve_model_alias("sonnet"),
            client.provider_id().as_deref(),
            &super::super::judge::JudgeRoots::default(),
        );
        match field {
            "check" => {
                current.acceptance[0].check = serde_json::from_value(
                    serde_json::json!({"kind":"command","command":"false","cwd":"project_root"}),
                )
                .unwrap()
            }
            "covers" => current.acceptance[0].covers.push("REQ-CHANGED".into()),
            "unchanged" => {}
            _ => unreachable!(),
        }
        current.acceptance[0].judgment.verdict = JudgeDecision::Refuted;
        println!(
            "EVIDENCE #419 {field} BEFORE: current={:?}; current check={:?}; current covers={:?}; pinned check={:?}; pinned covers={:?}",
            current.acceptance[0].judgment.verdict,
            current.acceptance[0].check,
            current.acceptance[0].covers,
            pinned.acceptance[0].check,
            pinned.acceptance[0].covers
        );
        let restored = super::super::incremental::keep_pinned_entries(
            &mut current,
            &pinned,
            &client,
            &super::super::judge::JudgeRoots::default(),
        );
        println!(
            "EVIDENCE #419 {field} AFTER: restored={}; verdict={:?}",
            restored.contains("AC-1"),
            current.acceptance[0].judgment.verdict
        );
        assert_eq!(restored.contains("AC-1"), field == "unchanged");
        assert_eq!(
            current.acceptance[0].judgment.verdict,
            if field == "unchanged" {
                JudgeDecision::Accepted
            } else {
                JudgeDecision::Refuted
            }
        );
    }
}

#[test]
fn pinned_restore_matches_the_batch_digest_past_the_shared_source_byte_budget() {
    let root = tempfile::tempdir().unwrap();
    let limit = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    let content = "x".repeat(limit * 3 / 4);
    std::fs::write(root.path().join("first"), &content).unwrap();
    std::fs::write(root.path().join("second"), &content).unwrap();
    let roots = super::super::judge::JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    let mut current = cache_contract();
    let mut first = current.acceptance[0].clone();
    first.id = "AC-X-001".into();
    first.check = serde_json::from_value(serde_json::json!({
        "kind": "command", "command": "operation first", "cwd": "project_root"
    }))
    .unwrap();
    let mut second = first.clone();
    second.id = "AC-X-002".into();
    second.check = serde_json::from_value(serde_json::json!({
        "kind": "command", "command": "operation second", "cwd": "project_root"
    }))
    .unwrap();
    current.acceptance = vec![first, second];
    current.supplementary.clear();
    let mut pinned = current.clone();
    let mut remaining = limit;
    for entry in &mut pinned.acceptance {
        entry.judgment.verdict = JudgeDecision::Accepted;
        entry.judgment.sampling = Some(serde_json::json!({
            "model": client.resolve_model_alias("sonnet"),
            "provider": client.provider_id(),
        }));
        crate::command::workflow_task_set::reauthor::cache::stamp_judgment_with_budget(
            entry,
            &client.resolve_model_alias("sonnet"),
            client.provider_id().as_deref(),
            &roots,
            &mut remaining,
        );
    }
    for entry in &mut current.acceptance {
        entry.judgment.verdict = JudgeDecision::Refuted;
    }

    let restored =
        super::super::incremental::keep_pinned_entries(&mut current, &pinned, &client, &roots);

    assert!(restored.contains("AC-X-002"));
    assert_eq!(
        current.acceptance[1].judgment.verdict,
        JudgeDecision::Accepted
    );
}
