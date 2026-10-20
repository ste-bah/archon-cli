use super::hermetic::{Unrun, data_digest, data_digest_scoped};
use super::*;

/// The state of the project data the site copies, for `tree`.
pub(super) fn data_state(probe: &HostProbe, tree: &Baseline) -> Result<Vec<String>, Unrun> {
    let mut states = probe.data_states.lock().expect("data states lock");
    if let Some(state) = states.get(&tree.repository) {
        return Ok(state.clone());
    }
    let state = match &probe.site {
        Site::Scratch(binding) => {
            let policy = &binding.policy;
            let project = archon_shell::paths::canonicalize(&policy.project).map_err(|error| {
                Unrun(format!(
                    "could not canonicalize project root {}: {error}",
                    policy.project.display()
                ))
            })?;
            let task_root =
                archon_shell::paths::canonicalize(&policy.task_root).map_err(|error| {
                    Unrun(format!(
                        "could not canonicalize task root {}: {error}",
                        policy.task_root.display()
                    ))
                })?;
            let task_root_relative = task_root
                .strip_prefix(&project)
                .map_err(|_| Unrun("scratch task root is outside the project root".into()))?;
            let mut state = policy
                .project_inputs
                .iter()
                .map(|input| {
                    data_digest_scoped(
                        &policy.project,
                        &policy.repository,
                        Some(task_root_relative),
                        input,
                        &policy.project_input_excludes,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            state.push(data_digest_scoped(
                &policy.project,
                &policy.repository,
                Some(task_root_relative),
                task_root_relative,
                &policy.project_input_excludes,
            )?);
            state
        }
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => vec![data_digest(
            &probe.project,
            &tree.repository,
            probe.task_root.as_deref(),
        )?],
    };
    states.insert(tree.repository.clone(), state.clone());
    Ok(state)
}
