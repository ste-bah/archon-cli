use std::path::Path;
pub(super) fn render(root: &Path) -> String {
    match read(root) {
        Ok(s) => s,
        Err(e) => format!("states: unreadable ({e:#})\n"),
    }
}
fn read(root: &Path) -> anyhow::Result<String> {
    let bytes =
        match std::fs::read(root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(e) => return Err(e.into()),
        };
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(&bytes)?;
    let acceptance_ids: std::collections::BTreeSet<String> =
        std::fs::read(root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE))
            .ok()
            .and_then(|b| {
                serde_json::from_slice::<archon_workflow::task_set_contract::AcceptanceContract>(&b)
                    .ok()
            })
            .map(|c| c.acceptance.into_iter().map(|c| c.id).collect())
            .unwrap_or_default();
    let mut counts = std::collections::BTreeMap::from([
        ("done", 0),
        ("missing", 0),
        ("undecided", 0),
        ("inspection", 0),
    ]);
    let mut out = String::new();
    for task in &skeleton.tasks {
        for (id, decision) in &task.requirement_states {
            *counts
                .get_mut(decision.state.as_str())
                .expect("known state") += 1;
            let proof = task.proven_done.iter().find(|p| &p.id == id);
            let deciding = proof.map_or("-", |p| p.check_id.as_str());
            let evidence = proof
                .map(|p| p.mutant_id.clone().unwrap_or_else(|| p.literals.join(", ")))
                .unwrap_or_else(|| "-".into());
            let notes = if decision.notes.is_empty() {
                String::new()
            } else {
                format!(" notes={}", decision.notes.join("; "))
            };
            let truncated = if proof.is_some_and(|p| p.evidence_truncated) {
                ", truncated"
            } else {
                ""
            };
            out.push_str(&format!(
                "state {} {id}: {} reason={} deciding={deciding} evidence={evidence}{truncated}{notes}\n",
                task.task_id,
                decision.state.as_str(),
                decision.reason
            ));
        }
    }
    if counts.values().sum::<usize>() == 0 {
        return Ok(out);
    }
    out.push_str(&format!(
        "states: done={} missing={} undecided={} inspection={}\n",
        counts["done"], counts["missing"], counts["undecided"], counts["inspection"]
    ));
    for task in &skeleton.tasks {
        for (id, decision) in &task.requirement_states {
            if decision.state == archon_workflow::requirement_state::RequirementState::Undecided {
                out.push_str(&format!(
                    "undecided {} {id}: {}{}\n",
                    task.task_id,
                    decision.reason,
                    if acceptance_ids.contains(id) {
                        " hops=n/a"
                    } else {
                        ""
                    }
                ));
            }
        }
    }
    Ok(out)
}
