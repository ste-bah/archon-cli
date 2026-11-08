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
            let generic_path_route = (line
            .contains("the command names no existing file path under the selected root")
            || line.contains(
                "the command names no file that the host can move aside under the selected root",
            ))
            && line.contains("A test, target or module name is not a file path")
            && (line.contains("Put the relative path of the file that decides the criterion")
                || line.contains("A path the check reads by its literal relative path, as an operand of a command that reads it (an existence test included), is a move-aside handle:"));
            !(line.contains(super::executability::CANNOT_FAIL)
                || line.contains("passed on the pre-implementation tree")
                || line.contains("passed on the selected tree"))
                || !line.contains("considered inputs:")
                || !(legacy_route || generic_path_route || (line.contains("name a file that exists there") || line.contains("name the real path")))
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
        "\nescalated to the PRD owner for review: {ids} still pass(es) on the probed tree after every repair; the harness gave the repair route {attempt} time(s). Last exclusion reasons: {exclusions}. Reassess the criterion only if every candidate was excluded for a reason other than check-generated data or selected-root choice; nothing unproven is published"
    ))
}

#[cfg(test)]
impl super::super::executability::HostProbe {
    #[cfg(unix)]
    pub(crate) fn assert_p1_prompt(text: &str) {
        let set = super::super::republish::test_fixture::frozen_set_proven(&[(
            "AC-X-001", "true", false,
        )]);
        let scope = super::AuthorScope::for_task_set_without_cache(
            set.project.path(),
            &set.tasks,
            &set.prd,
        )
        .unwrap();
        let prompt =
            super::author::author_prompt(&scope, &set.contract().acceptance[0], &[text.into()], 1);
        assert_eq!(prompt.matches(text).count(), 1, "{prompt}");
        assert!(
            !prompt.contains("The criterion already holds on the pre-implementation tree."),
            "{prompt}"
        );
        assert!(
            prompt.contains(
                "Make the check fail on the pre-implementation tree if it does not exercise what the work must add."
            ),
            "{prompt}"
        );
        assert!(
            prompt.contains("Otherwise, write a regression guard"),
            "{prompt}"
        );
    }

    #[cfg(unix)]
    pub(crate) fn assert_p1_readers(text: &str, baseline: bool) {
        use crate::command::workflow_gate::{GateFinding, GateId};
        assert_eq!(
            super::author::is_baseline_pass_note(text),
            baseline,
            "{text}"
        );
        let finding = GateFinding::new(
            GateId::FreezeAcceptance,
            text,
            "AC-X-001",
            None,
            archon_workflow::RemediationScope::CandidateArtifact,
        );
        assert_eq!(
            super::super::findings::is_baseline_pass_finding(&finding),
            baseline,
            "{text}"
        );
        assert_eq!(
            baseline_escalation(&BTreeSet::from(["AC-X-001".into()]), &[text.into()], 3).is_some(),
            baseline,
            "{text}"
        );
    }
}
