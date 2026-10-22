use std::collections::{BTreeMap, BTreeSet};

use super::mutation;

pub(crate) fn baseline_pass_finding(id: &str, short: &str) -> String {
    format!(
        "{}; data the check writes itself is never evidence; movable inputs must exist under the check's selected root before the run; to guard behaviour implemented in code, select the root that contains the existing implementation or test dependency and name it relative to that root; the check must still fail on its own assertion when that input is moved aside",
        baseline_pass_lead(id, short)
    )
}

pub(crate) fn baseline_pass_finding_without_discovered_input(id: &str, short: &str) -> String {
    format!(
        "{}; the command names no existing file path under the selected root. A test, target or module name is not a file path. Put the relative path of the file that decides the criterion literally in the command text, so that moving that file aside makes the check fail",
        baseline_pass_lead(id, short)
    )
}

fn baseline_pass_lead(id: &str, short: &str) -> String {
    format!(
        "check '{id}': the criterion already holds on the pre-implementation tree; write a regression guard. It {} at {short}, before any implementation, so it cannot show its criterion false and proves nothing",
        mutation::CANNOT_FAIL
    )
}

pub(crate) fn describe_candidates(candidates: &[mutation::CandidateDiagnostic]) -> String {
    describe_candidates_with_run_state(
        candidates,
        &BTreeSet::new(),
        &[],
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeMap::new(),
        false,
    )
}

pub(crate) fn describe_candidates_with_run_state(
    candidates: &[mutation::CandidateDiagnostic],
    generated: &BTreeSet<String>,
    recreated: &[String],
    moved: &BTreeSet<String>,
    original_recreated: &BTreeSet<String>,
    move_states: &BTreeMap<String, String>,
    run_observed: bool,
) -> String {
    if candidates.is_empty() {
        return "none discovered".into();
    }
    let mut grouped = BTreeMap::<&str, Vec<String>>::new();
    for candidate in candidates {
        let was_moved =
            moved.contains(&candidate.path) || original_recreated.contains(&candidate.path);
        let reason = if was_moved
            && (recreated.contains(&candidate.path) || generated.contains(&candidate.path))
        {
            mutation::ExclusionReason::CheckRecreatedPathObserved.label()
        } else if run_observed
            && (candidate.exclusion.is_none()
                || candidate.exclusion == Some(mutation::ExclusionReason::RepositoryCopyAlias)
                || candidate.exclusion == Some(mutation::ExclusionReason::MissingAtRoot))
            && !was_moved
        {
            match move_states.get(&candidate.path).map(String::as_str) {
                Some("absent at the selected root") => "absent at the selected root",
                Some("refused: symlink component") => "refused: symlink component",
                Some(state) if state.starts_with("move failed: ") => state,
                _ => "move failed: no move state was recorded",
            }
        } else {
            candidate
                .exclusion
                .map_or("eligible movable input", mutation::ExclusionReason::label)
        };
        let path = if was_moved {
            format!("{} (moved in the site copy)", candidate.path)
        } else {
            candidate.path.clone()
        };
        grouped.entry(reason).or_default().push(path);
    }
    grouped
        .into_iter()
        .map(|(reason, paths)| format!("{reason}: {}", paths.join(", ")))
        .collect::<Vec<_>>()
        .join("; ")
}
