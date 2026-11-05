use std::collections::{BTreeMap, BTreeSet};

use super::{BaselineOrigin, mutation};

pub(crate) fn baseline_pass_finding_for_candidates(
    id: &str,
    short: &str,
    candidates: &[mutation::CandidateDiagnostic],
) -> String {
    use mutation::ExclusionReason::*;
    let mut finding = baseline_pass_finding_without_discovered_input(id, short);
    let mut reasons = BTreeSet::new();
    for candidate in candidates {
        let clause = match candidate.exclusion {
            Some(ManifestOrSetup) => "manifests never move".to_string(),
            Some(SiblingFromRoot) => {
                "a sibling tree other than ../<repository dir>/ is never moved".into()
            }
            Some(OutsideCopiedTree) => {
                format!("`{}` is a path outside the copied tree", candidate.path)
            }
            Some(CheckWritesPathInText) => {
                format!("`{}` is a path the check itself writes", candidate.path)
            }
            Some(RepositoryAliasUnverified) => format!(
                "the host could not verify `{}` as the repository alias",
                candidate.path
            ),
            _ => continue,
        };
        reasons.insert(clause);
    }
    for reason in reasons {
        finding.push_str(&format!("; {reason}"));
    }
    finding
}

pub(crate) fn baseline_pass_finding(id: &str, short: &str) -> String {
    format!(
        "{} data the check writes itself is never evidence; movable inputs must exist under the check's selected root before the run; to guard behaviour implemented in code, select the root that contains the existing implementation or test dependency and name it relative to that root; the check must fail whenever the deciding test fails",
        baseline_pass_lead(id, short)
    )
}

pub(crate) fn baseline_pass_finding_without_discovered_input(id: &str, short: &str) -> String {
    format!(
        "{} the command names no file that the host can move aside under the selected root. A test, target or module name is not a file path. A program or script the check runs is never moved. Neither are a directory it changes into, values of -C/--manifest-path/--directory/--cwd/--project/--prefix, paths classified as command output (echo/printf/fail/die), the repository root or a whole directory tree at or above the selected root. The deciding file must be inside the selected root or inside the repository reached from project_root through ../<repository dir>/; a file in any other sibling tree is never moved. A path the check reads by its literal relative path, as an operand of a command that reads it (an existence test included), is a move-aside handle: the host may move it aside to prove the check can fail. A handle is not the deciding assertion; the test or implementation run whose status decides the check must still decide it. Use a handle, for example `test -f <path>` before the run that the file decides; the considered inputs that follow list each excluded path and its reason",
        baseline_pass_lead(id, short)
    )
}

pub(super) fn baseline_pass_lead(id: &str, short: &str) -> String {
    baseline_pass_lead_for_tree(id, short, BaselineOrigin::RecordedBase)
}

pub(super) fn baseline_pass_lead_for_tree(id: &str, short: &str, origin: BaselineOrigin) -> String {
    let tree = origin.tree_name();
    let advice = match origin {
        BaselineOrigin::RecordedBase => {
            "Either it does not exercise what the work must add (make it fail on that tree), or the criterion already holds there and it must be proven a regression guard"
        }
        _ => "The check must fail on the pre-implementation tree, or be proven a regression guard",
    };
    let cannot_fail = mutation::CANNOT_FAIL;
    format!("check '{id}': it passed on the {tree} at {short}, {cannot_fail}. {advice}:")
}

pub(crate) fn describe_candidates(candidates: &[mutation::CandidateDiagnostic]) -> String {
    describe_candidates_with_run_state(
        candidates,
        &BTreeSet::new(),
        &[],
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeMap::new(),
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
    covered_inputs: &BTreeMap<String, String>,
    run_observed: bool,
) -> String {
    if candidates.is_empty() {
        return "none discovered".into();
    }
    let mut grouped = BTreeMap::<&str, Vec<String>>::new();
    for candidate in candidates {
        let was_moved =
            moved.contains(&candidate.path) || original_recreated.contains(&candidate.path);
        let reason = if was_moved && recreated.contains(&candidate.path) {
            mutation::ExclusionReason::CheckRecreatedPathObserved.label()
        } else if was_moved && generated.contains(&candidate.path) {
            "present again after mutation; writer unknown"
        } else if was_moved && original_recreated.contains(&candidate.path) {
            // Only the parent's metadata changed while it was moved aside.
            "parent directory metadata changed while it was moved aside; writer unknown"
        } else if run_observed
            && (candidate.exclusion.is_none()
                || candidate.exclusion == Some(mutation::ExclusionReason::RepositoryCopyAlias)
                || candidate.exclusion == Some(mutation::ExclusionReason::MissingAtRoot))
            && !was_moved
        {
            match move_states.get(&candidate.path).map(String::as_str) {
                Some(state) if state.starts_with("covered by moved ancestor '") => state,
                _ if covered_inputs.contains_key(&candidate.path) => {
                    // Keep a child's own move failure when its ancestor failed.
                    move_states
                        .get(&candidate.path)
                        .map(String::as_str)
                        .unwrap_or("move failed: no move state was recorded")
                }
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

/// Change only host-authored wording; never rewrite captured check output.
pub(super) fn label_findings(
    findings: &mut BTreeMap<String, String>,
    short: &str,
    origin: BaselineOrigin,
) {
    for (id, finding) in findings {
        let old = baseline_pass_lead(id, short);
        if let Some(tail) = finding.strip_prefix(&old) {
            *finding = format!("{}{tail}", baseline_pass_lead_for_tree(id, short, origin));
        }
    }
}
