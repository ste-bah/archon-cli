//! Conservative resource scheduling from declared command lines.

/// Commands with no shared build state may retain the configured width.
/// All other declared programs use one lane, including unrecognized tools.
pub fn needs_serial_lane(text: &str) -> bool {
    // Shell substitutions and redirections can hide a writer behind an
    // otherwise read-only program. Unknown shell effects take the safe lane.
    if text.contains(['`', '>']) || text.contains("$(") || text.contains("<(") {
        return true;
    }
    text.split([';', '|', '&', '\n']).any(|part| {
        let mut words = part.split_whitespace();
        let program = words.find(|word| !word.contains('='));
        let Some(program) = program else { return false };
        let program = program.trim_matches(['\'', '"', '`']);
        let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
        let name = name.strip_suffix(".exe").unwrap_or(name);
        !matches!(
            name,
            "rg" | "grep"
                | "cat"
                | "ls"
                | "head"
                | "tail"
                | "echo"
                | "printf"
                | "pwd"
                | "true"
                | "false"
                | "test"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_share_a_lane_regardless_of_ecosystem() {
        for command in [
            "cargo test",
            "npm test",
            "pytest a b",
            "go test ./...",
            "./gradlew test",
            "mvn test",
            "dotnet test",
            "custom-check --all",
            "MODE=test /tools/compiler build",
            "cat file; make test",
            "echo $(custom-build check)",
            "echo `custom-build check`",
            "printf data > shared-output",
        ] {
            assert!(needs_serial_lane(command), "{command}");
        }
    }
    #[test]
    fn mentions_and_inspections_do_not_claim_build_resources() {
        for command in [
            "echo cargo test",
            "rg 'cargo ' README.md",
            "cat package.json",
            "",
        ] {
            assert!(!needs_serial_lane(command), "{command}");
        }
    }
}
