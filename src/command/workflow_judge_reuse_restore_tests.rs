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
            "logic_identity": crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity().unwrap()
        }));
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
        let restored =
            super::super::incremental::keep_pinned_entries(&mut current, &pinned, &client);
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
            "logic_identity": crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity().unwrap()
        }));
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
        let restored =
            super::super::incremental::keep_pinned_entries(&mut current, &pinned, &client);
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
