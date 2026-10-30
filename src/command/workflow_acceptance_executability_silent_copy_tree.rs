use super::*;

/// Issue 420: the copy `probe`'s checks run in, as (depth of a check's
/// working directory below the copy's top, the names the top holds). A
/// scratch site holds `repo`, `project` and its own directories
/// (`acceptance_scratch::ScratchRoots::prepare`); the probe's own copy
/// holds `repo` and `project` (`HermeticCopy::at`), or keeps a project its
/// repository tracks inside `repo`, at its depth there. The live tree is no
/// copy (`None`): a path above a check's directory may be real there.
/// Issue 495: the scratch names come from the site's own list, with the
/// build target placed as the run's build cache placed it.
pub(super) fn scratch_copy_tree(
    probe: &HostProbe,
    contract: &AcceptanceContract,
    layout: Option<&super::hermetic::CopyLayout>,
) -> Option<(usize, Vec<String>)> {
    match &probe.site {
        Site::Scratch(binding) => {
            let build_cache = layout
                .and_then(|layout| layout.site_build_cache)
                .unwrap_or(binding.policy.build_cache.is_some());
            let mut roots: Vec<String> =
                archon_workflow::acceptance_scratch::site_internal_directories(build_cache)
                    .into_iter()
                    .map(str::to_string)
                    .collect();
            roots.extend(layout.map_or_else(
                || {
                    hermetic::siblings::copied_root_names(
                        &probe.project,
                        &probe.repository,
                        contract,
                    )
                },
                |layout| {
                    layout
                        .siblings
                        .iter()
                        .filter_map(|sibling| {
                            sibling
                                .path
                                .components()
                                .find_map(|component| match component {
                                    std::path::Component::Normal(name) => {
                                        Some(name.to_string_lossy().into_owned())
                                    }
                                    _ => None,
                                })
                        })
                        .collect()
                },
            ));
            Some((1, roots))
        }
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => None,
    }
}
