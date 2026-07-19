pub(crate) fn apply_workflow_resource_defaults(env: &mut Vec<(String, String)>, command: &str) {
    ensure_env_default(env, "ARCHON_WORKFLOW_RESOURCE_CLASS", "constrained");
    if contains_shell_word(command, "cargo") {
        ensure_env_default(env, "CARGO_BUILD_JOBS", "1");
        ensure_env_default(env, "CARGO_INCREMENTAL", "0");
    }
}

fn ensure_env_default(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if env.iter().any(|(existing, _)| existing == key) {
        return;
    }
    env.push((key.to_string(), value.to_string()));
}

fn contains_shell_word(command: &str, needle: &str) -> bool {
    command.match_indices(needle).any(|(idx, _)| {
        let before = command[..idx].chars().next_back();
        let after = command[idx + needle.len()..].chars().next();
        !is_word_char(before) && !is_word_char(after)
    })
}

fn is_word_char(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_commands_get_constrained_defaults() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "cargo test -p demo");

        assert!(env.contains(&(
            "ARCHON_WORKFLOW_RESOURCE_CLASS".to_string(),
            "constrained".to_string()
        )));
        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "1".to_string())));
        assert!(env.contains(&("CARGO_INCREMENTAL".to_string(), "0".to_string())));
    }

    #[test]
    fn non_cargo_commands_get_only_generic_resource_class() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "npm test");

        assert!(
            env.iter()
                .any(|(key, _)| key == "ARCHON_WORKFLOW_RESOURCE_CLASS")
        );
        assert!(!env.iter().any(|(key, _)| key == "CARGO_BUILD_JOBS"));
    }

    #[test]
    fn explicit_env_values_are_preserved() {
        let mut env = vec![("CARGO_BUILD_JOBS".to_string(), "4".to_string())];

        apply_workflow_resource_defaults(&mut env, "cargo test");

        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "4".to_string())));
        assert!(!env.contains(&("CARGO_BUILD_JOBS".to_string(), "1".to_string())));
    }

    #[test]
    fn shell_word_detection_ignores_substrings() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "echo xcargo");

        assert!(!env.iter().any(|(key, _)| key == "CARGO_BUILD_JOBS"));
    }
}
