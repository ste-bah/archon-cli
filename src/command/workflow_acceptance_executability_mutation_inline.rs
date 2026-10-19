//! Program text passed to an inline-program flag.

/// The position and text of each program passed to an inline-program flag.
pub(super) fn programs(commands: &[Vec<String>]) -> Vec<(usize, usize, String)> {
    archon_workflow::acceptance_sibling_paths::inline_programs(commands)
}

pub(super) fn program_positions(
    commands: &[Vec<String>],
) -> std::collections::BTreeSet<(usize, usize)> {
    programs(commands)
        .into_iter()
        .map(|(command, argument, _)| (command, argument))
        .collect()
}
