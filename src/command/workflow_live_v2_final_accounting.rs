use super::*;

pub(crate) fn reconcile_final_task_statuses(
    report: &mut WorkflowV2FinalReport,
    required_task_ids: &[String],
) {
    let required = required_task_ids.iter().cloned().collect::<BTreeSet<_>>();
    report.accepted_tasks.retain(|id| required.contains(id));
    report.noop_tasks.retain(|id| required.contains(id));
    let noop = report.noop_tasks.iter().cloned().collect::<BTreeSet<_>>();
    report.accepted_tasks.retain(|id| !noop.contains(id));
    let completed = report
        .accepted_tasks
        .iter()
        .chain(report.noop_tasks.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    report
        .blocked_tasks
        .retain(|id| required.contains(id) && !completed.contains(id));
    let blocked = report
        .blocked_tasks
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    report
        .failed_tasks
        .retain(|id| required.contains(id) && !completed.contains(id) && !blocked.contains(id));
    let failed = report.failed_tasks.iter().cloned().collect::<BTreeSet<_>>();
    report.missing_tasks.retain(|id| {
        required.contains(id)
            && !completed.contains(id)
            && !blocked.contains(id)
            && !failed.contains(id)
    });
    report.review_blockers = unique_final_gaps(&report.residual_gaps);
}

fn unique_final_gaps(gaps: &[WorkflowV2ResidualGap]) -> Vec<WorkflowV2ResidualGap> {
    let mut seen = BTreeSet::new();
    gaps.iter()
        .filter(|gap| seen.insert(gap.id.clone()))
        .cloned()
        .collect()
}
