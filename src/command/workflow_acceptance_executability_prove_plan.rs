use super::mutation::Markers;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;
use std::collections::BTreeMap;

pub(super) struct BuiltMutationPlan {
    pub(super) plan: archon_workflow::acceptance_scratch::MutationPlan,
    pub(super) covered_inputs: BTreeMap<String, String>,
}

pub(super) fn plan(
    names: &[String],
    discovery: &mutation::InputDiscovery,
    entry: &AcceptanceCriterion,
    markers: &Markers,
) -> BuiltMutationPlan {
    let root_name = match &entry.check {
        AcceptanceCheck::Command {
            cwd: TrustedCwd::RepoRoot,
            ..
        } => "repo",
        _ => "project",
    };
    let inputs: Vec<_> = names
        .iter()
        .filter_map(|name| {
            let candidate = discovery
                .candidates
                .iter()
                .find(|candidate| &candidate.path == name)
                .expect("eligible input has a candidate");
            let (relative_root, relative_input) = match (
                candidate.move_path.as_deref(),
                candidate.move_root.as_deref(),
            ) {
                (Some(path), Some(root)) => (
                    "repo",
                    Path::new(path)
                        .strip_prefix(root)
                        .expect("alias move path is beneath its copy root"),
                ),
                _ => (root_name, Path::new(name)),
            };
            debug_assert!(
                !relative_input.as_os_str().is_empty(),
                "input discovery excludes empty paths"
            );
            if relative_input.as_os_str().is_empty() {
                return None;
            }
            let move_path = PathBuf::from(relative_root).join(relative_input);
            let aside_path = PathBuf::from(format!(
                "{}.archon-mutated-{}",
                move_path.display(),
                markers
                    .nonce()
                    .get(..12)
                    .expect("mutation nonce is 32 hex digits")
            ));
            Some(archon_workflow::acceptance_scratch::MutationInputPlan {
                named_path: name.clone(),
                site_copy_alias: candidate.move_path.is_some(),
                move_path,
                aside_path,
            })
        })
        .collect();
    let all_inputs = inputs;
    let mut retained_inputs = all_inputs.clone();
    retained_inputs.sort_by_key(|input| input.move_path.components().count());
    let covered_inputs = all_inputs
        .iter()
        .filter_map(|input| {
            retained_inputs
                .iter()
                .find(|ancestor| {
                    archon_workflow::acceptance_scratch::is_strict_descendant(
                        &input.move_path,
                        &ancestor.move_path,
                    )
                })
                .map(|ancestor| (input.named_path.clone(), ancestor.named_path.clone()))
        })
        .collect();
    BuiltMutationPlan {
        plan: archon_workflow::acceptance_scratch::MutationPlan {
            nonce: markers.nonce().to_string(),
            inputs: retained_inputs,
        },
        covered_inputs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ancestor_directory_covers_a_preceding_child_input() {
        let names = vec!["d/f".to_string(), "d".to_string()];
        let discovery = mutation::InputDiscovery {
            eligible: names.clone(),
            candidates: names
                .iter()
                .map(|path| mutation::CandidateDiagnostic {
                    path: path.clone(),
                    move_path: None,
                    move_root: None,
                    exclusion: None,
                })
                .collect(),
        };
        let entry: AcceptanceCriterion = serde_json::from_value(serde_json::json!({
            "id": "AC-X-001",
            "criterion": "the command checks its input",
            "check": {"kind": "command", "command": "test -f d/f && test -d d", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "test"}
        }))
        .unwrap();
        let markers = mutation::Markers::with_nonce("0123456789abcdef0123456789abcdef").unwrap();
        let built = plan(&names, &discovery, &entry, &markers);
        assert_eq!(built.plan.inputs.len(), 2);
        assert_eq!(built.plan.inputs[0].named_path, "d");
        assert_eq!(built.plan.inputs[0].move_path, PathBuf::from("project/d"));
        assert_eq!(
            built.covered_inputs.get("d/f").map(String::as_str),
            Some("d")
        );
    }

    #[test]
    fn parent_components_do_not_make_a_path_an_ancestor_child() {
        let names = vec!["d/../e/f".to_string(), "d".to_string()];
        let discovery = mutation::InputDiscovery {
            eligible: names.clone(),
            candidates: names
                .iter()
                .map(|path| mutation::CandidateDiagnostic {
                    path: path.clone(),
                    move_path: None,
                    move_root: None,
                    exclusion: None,
                })
                .collect(),
        };
        let entry: AcceptanceCriterion = serde_json::from_value(serde_json::json!({
            "id": "AC-X-001",
            "criterion": "the command checks its input",
            "check": {"kind": "command", "command": "test -f d/../e/f && test -d d", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "test"}
        }))
        .unwrap();
        let markers = mutation::Markers::with_nonce("0123456789abcdef0123456789abcdef").unwrap();
        let built = plan(&names, &discovery, &entry, &markers);
        assert_eq!(built.plan.inputs.len(), 2);
        assert!(built.covered_inputs.is_empty());
    }
}
