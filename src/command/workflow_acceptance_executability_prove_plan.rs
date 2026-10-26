use super::mutation::Markers;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

pub(super) fn plan(
    names: &[String],
    discovery: &mutation::InputDiscovery,
    entry: &AcceptanceCriterion,
    markers: &Markers,
) -> archon_workflow::acceptance_scratch::MutationPlan {
    let root_name = match &entry.check {
        AcceptanceCheck::Command {
            cwd: TrustedCwd::RepoRoot,
            ..
        } => "repo",
        _ => "project",
    };
    archon_workflow::acceptance_scratch::MutationPlan {
        nonce: markers.nonce().to_string(),
        inputs: names
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
            .collect(),
    }
}
