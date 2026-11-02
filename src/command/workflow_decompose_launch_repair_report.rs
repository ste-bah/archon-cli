use super::*;

pub(super) fn repair_report(
    result: crate::command::workflow_task_set::republish::ReauthorResult,
    decision: &LaunchAcceptance,
) -> Result<(String, serde_json::Value)> {
    let ids: BTreeSet<String> = decision.reauthor.keys().cloned().collect();
    for diagnostic in &result.diagnostics {
        eprintln!("{diagnostic}");
    }
    for (id, reason) in &result.unproven {
        let owner = match result.authored_pending.contains_key(id) {
            true => "its author",
            false => "the host",
        };
        eprintln!("acceptance check {id} remains pending for {owner}: {reason}");
    }
    let published = ids
        .iter()
        .filter(|id| !result.unproven.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let pending = result.unproven.keys().cloned().collect::<Vec<_>>();
    let authored_pending = (pending.iter())
        .filter(|id| result.authored_pending.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let line = format!(
        "frozen_chain_launch acceptance=reauthored published_ids={} pending_ids={} acceptance_digest={} freeze_event={} skeleton_digest={}",
        published.join(","),
        pending.join(","),
        result.acceptance_digest,
        result.freeze_event_id,
        result.skeleton_digest.as_deref().unwrap_or("none")
    );
    let mut detail = serde_json::json!({
        "event": REAUTHORED_EVENT,
        "ids": ids,
        "published_ids": published,
        "pending_ids": pending,
        "authored_pending_ids": authored_pending,
        "refusals": decision.reauthor,
        "recorded_logic": decision.recorded,
        "acceptance_digest": result.acceptance_digest,
        "freeze_event_id": result.freeze_event_id,
        "skeleton_digest": result.skeleton_digest,
        "unproven": result.unproven,
    });
    if !result.host_faults.is_empty() {
        detail["host_faults"] = serde_json::to_value(result.host_faults)?;
    }
    Ok((line, detail))
}
