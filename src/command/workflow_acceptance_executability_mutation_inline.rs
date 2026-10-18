//! Program text passed to an inline-program flag.

const INLINE_PROGRAM_FLAGS: [&str; 5] = ["-c", "-e", "--eval", "--command", "--exec"];

/// The position and text of each program passed to an inline-program flag.
pub(super) fn programs(commands: &[Vec<String>]) -> Vec<(usize, usize, String)> {
    let mut found = Vec::new();
    for (command_index, command) in commands.iter().enumerate() {
        let mut argument_index = 1;
        while let Some(argument) = command.get(argument_index) {
            if INLINE_PROGRAM_FLAGS.contains(&argument.as_str()) {
                if let Some(program) = command.get(argument_index + 1) {
                    found.push((command_index, argument_index + 1, program.clone()));
                }
                argument_index += 2;
            } else {
                argument_index += 1;
            }
        }
    }
    found
}

pub(super) fn program_positions(
    commands: &[Vec<String>],
) -> std::collections::BTreeSet<(usize, usize)> {
    programs(commands)
        .into_iter()
        .map(|(command, argument, _)| (command, argument))
        .collect()
}
