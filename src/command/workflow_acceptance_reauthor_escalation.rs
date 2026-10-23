use std::collections::BTreeSet;

pub(super) fn baseline_escalation(
    pending: &BTreeSet<String>,
    lines: &[String],
    attempt: usize,
) -> Option<String> {
    let route = [
        "data the check writes itself is never evidence",
        "movable inputs must exist under the check's selected root before the run",
        "select the root that contains the existing implementation or test dependency",
        "the check must fail whenever the deciding test fails",
    ];
    let notes: Vec<&String> = pending.iter().zip(lines).map(|(_, line)| line).collect();
    if notes.len() != pending.len()
        || notes.iter().any(|line| {
            let legacy_route = route.iter().all(|fact| line.contains(fact));
            let generic_path_route = line.contains("the command names no existing file path under the selected root")
                && line.contains("A test, target or module name is not a file path")
                && line.contains("Put the relative path of the file that decides the criterion literally in the command text");
            !line.contains(super::executability::CANNOT_FAIL)
                || !line.contains("considered inputs:")
                || !(legacy_route || generic_path_route)
                || line.contains("could not run")
        })
    {
        return None;
    }
    let ids = pending
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let exclusions = notes
        .iter()
        .filter_map(|line| {
            line.split_once("considered inputs:")
                .map(|(_, reasons)| reasons)
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "\nescalated to the PRD owner for review: {ids} still pass(es) on the pre-implementation tree after every repair; the harness gave the repair route {attempt} time(s). Last exclusion reasons: {exclusions}. Reassess the criterion only if every candidate was excluded for a reason other than check-generated data or selected-root choice; nothing unproven is published"
    ))
}
