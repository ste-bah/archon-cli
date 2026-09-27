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
