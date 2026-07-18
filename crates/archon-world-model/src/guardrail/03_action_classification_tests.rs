#[test]
fn task_classification_uses_surface_not_prompt_prose() {
    assert_eq!(
        classify_task(
            "coding pipeline: implement authentication and research citations",
            WorldAdvisorSurface::PipelineStep,
        ),
        RuntimeTaskClass::PipelineExecution,
    );
    assert_eq!(
        classify_task("fix failing tests", WorldAdvisorSurface::InteractiveSession),
        RuntimeTaskClass::GeneralAnswer,
    );
    assert_eq!(
        classify_task("anything", WorldAdvisorSurface::CodingTask),
        RuntimeTaskClass::CodingChange,
    );
}

#[test]
fn tool_action_classification_uses_structured_tool_identity() {
    let empty = serde_json::json!({});

    assert_eq!(
        classify_tool_action("Edit", &empty, WorldAdvisorSurface::InteractiveSession),
        RuntimeTaskClass::CodingChange,
    );
    assert_eq!(
        classify_tool_action("WebSearch", &empty, WorldAdvisorSurface::InteractiveSession),
        RuntimeTaskClass::ResearchAnswer,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "cargo test -p archon-world-model"}),
            WorldAdvisorSurface::InteractiveSession,
        ),
        RuntimeTaskClass::VerificationOnly,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "git push origin main"}),
            WorldAdvisorSurface::InteractiveSession,
        ),
        RuntimeTaskClass::ExternalSideEffect,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "rm generated-output"}),
            WorldAdvisorSurface::InteractiveSession,
        ),
        RuntimeTaskClass::DataMutation,
    );
}

#[test]
fn tool_action_classifies_each_unquoted_shell_chain_segment() {
    let surface = WorldAdvisorSurface::InteractiveSession;

    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "cargo test && git push origin main"}),
            surface,
        ),
        RuntimeTaskClass::ExternalSideEffect,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "cargo test; rm -rf generated"}),
            surface,
        ),
        RuntimeTaskClass::DataMutation,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "echo done | rm target"}),
            surface,
        ),
        RuntimeTaskClass::DataMutation,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "cargo test & git push origin main"}),
            surface,
        ),
        RuntimeTaskClass::ExternalSideEffect,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "echo rm\\ target; git push origin main"}),
            surface,
        ),
        RuntimeTaskClass::ExternalSideEffect,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "echo \\\"rm target; git push\\\""}),
            surface,
        ),
        RuntimeTaskClass::GeneralAnswer,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "echo 'rm target | git push'"}),
            surface,
        ),
        RuntimeTaskClass::GeneralAnswer,
    );
}

#[test]
fn tool_action_classifies_unquoted_nested_shell_substitutions() {
    let surface = WorldAdvisorSurface::InteractiveSession;

    for (command, expected) in [
        (
            "echo $(git push origin main)",
            RuntimeTaskClass::ExternalSideEffect,
        ),
        ("echo `rm -rf generated`", RuntimeTaskClass::DataMutation),
        (
            "echo '$(git push origin main)'",
            RuntimeTaskClass::GeneralAnswer,
        ),
        ("echo '`rm -rf generated`'", RuntimeTaskClass::GeneralAnswer),
        (
            "echo $(git push origin main",
            RuntimeTaskClass::GeneralAnswer,
        ),
        ("echo `rm -rf generated", RuntimeTaskClass::GeneralAnswer),
    ] {
        assert_eq!(
            classify_tool_action("Bash", &serde_json::json!({"command": command}), surface),
            expected,
            "{command}",
        );
    }
}

#[test]
fn tool_action_bounds_nested_shell_substitution_depth() {
    let within_bound = format!(
        "echo {}git push origin main{}",
        "$(".repeat(8),
        ")".repeat(8),
    );
    let beyond_bound = format!(
        "echo {}git push origin main{}",
        "$(".repeat(9),
        ")".repeat(9),
    );
    let surface = WorldAdvisorSurface::InteractiveSession;

    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": within_bound}),
            surface,
        ),
        RuntimeTaskClass::ExternalSideEffect,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": beyond_bound}),
            surface,
        ),
        RuntimeTaskClass::GeneralAnswer,
    );
}

#[test]
fn tool_action_normalizes_common_command_wrappers_and_package_options() {
    let surface = WorldAdvisorSurface::InteractiveSession;

    for command in [
        "npm --silent publish",
        "/usr/bin/npm --silent publish",
        "env npm publish",
        "env -i PATH=/usr/bin npm publish",
        "sudo -n git push origin main",
        "sudo -u root git push origin main",
        "command -- git push origin main",
    ] {
        assert_eq!(
            classify_tool_action("Bash", &serde_json::json!({"command": command}), surface),
            RuntimeTaskClass::ExternalSideEffect,
            "{command}",
        );
    }
    for command in [
        "env --unknown npm publish",
        "sudo -u git push origin main",
        "command --unknown git push origin main",
    ] {
        assert_eq!(
            classify_tool_action("Bash", &serde_json::json!({"command": command}), surface),
            RuntimeTaskClass::GeneralAnswer,
            "{command}",
        );
    }
}
#[test]
fn tool_action_normalizes_executables_and_unwraps_shell_commands() {
    let surface = WorldAdvisorSurface::InteractiveSession;

    for command in [
        "git -C /repo push origin main",
        "/usr/bin/git push origin main",
        r"C:\\Git\\bin\\git.exe push origin main",
        r"C:\\Git\\bin\\GIT.EXE push origin main",
    ] {
        assert_eq!(
            classify_tool_action("Bash", &serde_json::json!({"command": command}), surface),
            RuntimeTaskClass::ExternalSideEffect,
            "{command}",
        );
    }
    for command in [
        "/bin/rm generated",
        "bash -lc 'cargo test && git push origin main'",
    ] {
        assert_eq!(
            classify_tool_action("Bash", &serde_json::json!({"command": command}), surface),
            if command.starts_with("bash") {
                RuntimeTaskClass::ExternalSideEffect
            } else {
                RuntimeTaskClass::DataMutation
            },
            "{command}",
        );
    }
}

#[test]
fn tool_action_ignores_sql_keywords_inside_string_literals() {
    let surface = WorldAdvisorSurface::InteractiveSession;

    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "psql -c \"SELECT 'DELETE'\""}),
            surface,
        ),
        RuntimeTaskClass::GeneralAnswer,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "psql -c 'DROP TABLE archived_events'"}),
            surface,
        ),
        RuntimeTaskClass::DataMutation,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "psql -c 'SELECT $$DELETE$$'"}),
            surface,
        ),
        RuntimeTaskClass::GeneralAnswer,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "psql -c 'SELECT $tag$DROP$tag$'"}),
            surface,
        ),
        RuntimeTaskClass::GeneralAnswer,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "psql -c 'SELECT $$DELETE'"}),
            surface,
        ),
        RuntimeTaskClass::GeneralAnswer,
    );
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "psql -c 'DELETE FROM archived_events'"}),
            surface,
        ),
        RuntimeTaskClass::DataMutation,
    );
}

#[test]
fn tool_action_classifies_executable_sql_deletion() {
    assert_eq!(
        classify_tool_action(
            "Bash",
            &serde_json::json!({"command": "psql -c \"DROP TABLE archived_events\""}),
            WorldAdvisorSurface::InteractiveSession,
        ),
        RuntimeTaskClass::DataMutation,
    );
}

#[test]
fn tool_action_surface_overrides_take_priority() {
    assert_eq!(
        classify_tool_action(
            "Edit",
            &serde_json::json!({}),
            WorldAdvisorSurface::VerificationRun,
        ),
        RuntimeTaskClass::VerificationOnly,
    );
    assert_eq!(
        classify_tool_action(
            "WebSearch",
            &serde_json::json!({}),
            WorldAdvisorSurface::Pipeline,
        ),
        RuntimeTaskClass::PipelineExecution,
    );
}
