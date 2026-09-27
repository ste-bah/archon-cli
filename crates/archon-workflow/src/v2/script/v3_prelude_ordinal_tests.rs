//! Issue-122: a unit a resumed session skips takes its recorded place in the
//! call ordinal, but never reaches back into ids this session already used.

use super::tests::run_scripted;

const SCRIPT: &str = r#"export const meta = { name: 'ordinals', description: 'd', phases: [] }
const opts = { taskFileFor: () => 'tasks/T.md', targetFilesFor: () => ['src/t.txt'] }
const contests = await resolveContests(opts)
const after = await agent('after', { label: 'after', write: true, taskIds: ['TASK-Y'], targetFiles: ['src/t.txt'] })
return { contests }
"#;

fn entry(id: &str, declarer: &str, attempted: bool, resume: Option<u64>) -> serde_json::Value {
    serde_json::json!({"source": "host", "path": "p.txt", "state": "absent", "declarer": declarer,
        "deleted_by": "TASK-D", "deleted_in": "s", "confirmation_id": id, "attempted": attempted,
        "remediate": false, "resume_ordinal": resume})
}

#[tokio::test]
async fn a_skipped_unit_realigned_again_never_reaches_back_into_this_sessions_ids() {
    let (calls, _) = run_scripted(SCRIPT, |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        let plan = match id {
            // The skipped unit X ended at ordinal 30 in an earlier session;
            // Y is asked now and refused, so its remediation runs.
            "audit-contests-1" => vec![entry("confirm-x", "TASK-X", true, Some(30)), entry("confirm-y", "TASK-Y", false, None)],
            _ => vec![entry("confirm-x", "TASK-X", true, Some(30)), entry("confirm-y", "TASK-Y", true, None)],
        };
        if id.starts_with("audit-contests-") {
            return serde_json::json!({"status": "accepted", "summary": "plan", "audit_contests": plan});
        }
        if id == "verification-wave-confirm-y" {
            return serde_json::json!({"status": "needs_review", "summary": "refused"});
        }
        serde_json::json!({"status": "accepted", "summary": "ok",
            "result": {"status": "accepted", "summary": "ok"}, "patch_landed": true})
    })
    .await;
    let ordinal = |id: &str| id.rsplit('-').next().and_then(|n| n.parse::<u64>().ok());
    let ids: Vec<String> = calls
        .iter()
        .filter_map(|(_, payload)| payload["id"].as_str().map(str::to_string))
        .collect();
    let remediation: Vec<u64> = ids
        .iter()
        .filter(|id| id.contains("review-remediate-") || id.contains("review-verify-"))
        .filter_map(|id| ordinal(id))
        .collect();
    assert_eq!(
        remediation.first(),
        Some(&31),
        "Y's unit follows X's place: {ids:#?}"
    );
    let after = ids
        .iter()
        .find(|id| id.starts_with("after-"))
        .and_then(|id| ordinal(id))
        .unwrap();
    let highest = remediation.iter().max().copied().unwrap();
    assert!(
        after > highest,
        "a later call never takes an ordinal this session used: {after} <= {highest}: {ids:#?}"
    );
}

const RESIDUALS: &str = r#"export const meta = { name: 'ordinals', description: 'd', phases: [] }
const opts = { taskFileFor: () => 'tasks/T.md', targetFilesFor: () => ['src/t.txt'] }
const rounds = await resolveResiduals(opts)
const after = await agent('after', { label: 'after', write: true, taskIds: ['TASK-A'], targetFiles: ['src/t.txt'] })
return { rounds }
"#;

fn round(key: &str, fix_ordinal: Option<u64>) -> serde_json::Value {
    serde_json::json!({"source": "host", "key": key, "kind": "owned", "task_ids": ["TASK-A"],
        "expansion_files": [], "severity": "high", "claim": format!("Host round {key}: fix it"),
        "dispatchable": true, "attempted": false, "fix_ordinal": fix_ordinal})
}

/// A round re-entered after one this session ran files its fix where it was
/// filed, even below the ids the other round used (its own label keeps them
/// apart); nothing after reaches back into either.
#[tokio::test]
async fn a_re_entered_round_refiles_its_fix_below_a_round_this_session_ran() {
    let (calls, _) = run_scripted(RESIDUALS, |_, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "residual-gaps-1" {
            return serde_json::json!({"status": "accepted", "summary": "plan",
                "residual_plan": [round("residual-aaa", None), round("residual-bbb", Some(1))]});
        }
        if id.starts_with("residual-gaps-") {
            return serde_json::json!({"status": "accepted", "summary": "plan", "residual_plan": []});
        }
        serde_json::json!({"status": "accepted", "summary": "ok",
            "result": {"status": "accepted", "summary": "ok"}, "patch_landed": true})
    })
    .await;
    let ids: Vec<String> = calls
        .iter()
        .filter_map(|(_, payload)| payload["id"].as_str().map(str::to_string))
        .collect();
    let fix_of = |key: &str| {
        ids.iter()
            .find(|id| id.starts_with("review-remediate-") && id.contains(&key[9..12]))
            .cloned()
            .unwrap_or_else(|| panic!("{key}: {ids:#?}"))
    };
    assert!(fix_of("residual-aaa").ends_with("-1"), "{ids:#?}");
    assert!(
        fix_of("residual-bbb").ends_with("-1"),
        "refiled where it was filed: {ids:#?}"
    );
    let after = ids.iter().find(|id| id.starts_with("after-")).unwrap();
    assert!(after.ends_with("-3"), "{ids:#?}");
}
