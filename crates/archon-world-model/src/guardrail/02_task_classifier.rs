pub fn classify_task(_summary: &str, surface: WorldAdvisorSurface) -> RuntimeTaskClass {
    match surface {
        WorldAdvisorSurface::VerificationRun => RuntimeTaskClass::VerificationOnly,
        WorldAdvisorSurface::Pipeline | WorldAdvisorSurface::PipelineStep => {
            RuntimeTaskClass::PipelineExecution
        }
        WorldAdvisorSurface::CodingTask => RuntimeTaskClass::CodingChange,
        _ => RuntimeTaskClass::GeneralAnswer,
    }
}

pub fn classify_tool_action(
    tool_name: &str,
    input: &serde_json::Value,
    surface: WorldAdvisorSurface,
) -> RuntimeTaskClass {
    match surface {
        WorldAdvisorSurface::VerificationRun => return RuntimeTaskClass::VerificationOnly,
        WorldAdvisorSurface::Pipeline | WorldAdvisorSurface::PipelineStep => {
            return RuntimeTaskClass::PipelineExecution;
        }
        _ => {}
    }

    let tool_name = tool_name.to_ascii_lowercase();
    if is_delete_or_remove_tool(&tool_name) {
        return RuntimeTaskClass::DataMutation;
    }
    match tool_name.as_str() {
        "edit" | "write" | "notebookedit" => RuntimeTaskClass::CodingChange,
        "webfetch" | "websearch" => RuntimeTaskClass::ResearchAnswer,
        "bash" => classify_bash_command(input),
        _ if matches!(surface, WorldAdvisorSurface::CodingTask) => RuntimeTaskClass::CodingChange,
        _ => RuntimeTaskClass::GeneralAnswer,
    }
}

fn is_delete_or_remove_tool(tool_name: &str) -> bool {
    tool_name.starts_with("delete") || tool_name.starts_with("remove")
}

fn classify_bash_command(input: &serde_json::Value) -> RuntimeTaskClass {
    let Some(command) = input.get("command").and_then(serde_json::Value::as_str) else {
        return RuntimeTaskClass::GeneralAnswer;
    };
    let tokens = command_tokens(command);
    if is_destructive_command(&tokens) {
        RuntimeTaskClass::DataMutation
    } else if is_external_command(&tokens) {
        RuntimeTaskClass::ExternalSideEffect
    } else if is_verification_command(&tokens) {
        RuntimeTaskClass::VerificationOnly
    } else {
        RuntimeTaskClass::GeneralAnswer
    }
}

fn command_tokens(command: &str) -> Vec<&str> {
    command
        .split(|character: char| character.is_whitespace() || matches!(character, ';' | '|' | '&'))
        .filter(|token| !token.is_empty())
        .collect()
}

fn is_verification_command(tokens: &[&str]) -> bool {
    match tokens {
        ["cargo", operation, ..]
            if matches!(*operation, "test" | "check" | "build" | "clippy" | "fmt") =>
        {
            true
        }
        ["pytest", ..] => true,
        [package_manager, operation, target, ..]
            if matches!(*package_manager, "npm" | "pnpm" | "yarn" | "bun")
                && ((matches!(*operation, "test" | "run")
                    && matches!(*target, "test" | "build" | "lint" | "typecheck"))
                    || (*operation == "test" && *target == "--")) =>
        {
            true
        }
        [package_manager, "test", ..]
            if matches!(*package_manager, "npm" | "pnpm" | "yarn" | "bun") =>
        {
            true
        }
        _ => false,
    }
}

fn is_external_command(tokens: &[&str]) -> bool {
    matches!(tokens, ["git", "push", ..])
        || matches!(tokens, ["deploy", ..] | ["publish", ..])
        || matches!(
            tokens,
            [package_manager, operation, ..]
                if matches!(*package_manager, "npm" | "pnpm" | "yarn" | "bun")
                    && matches!(*operation, "publish" | "deploy")
        )
}

fn is_destructive_command(tokens: &[&str]) -> bool {
    matches!(tokens, ["rm", ..]) || has_executable_sql_mutation(tokens)
}

fn has_executable_sql_mutation(tokens: &[&str]) -> bool {
    let Some((database_client, arguments)) = tokens.split_first() else {
        return false;
    };
    matches!(
        *database_client,
        "psql" | "sqlite3" | "mysql" | "mariadb" | "sqlcmd"
    ) && arguments.iter().any(|argument| {
        let keyword = argument.trim_matches(|character: char| !character.is_ascii_alphabetic());
        matches!(keyword.to_ascii_uppercase().as_str(), "DROP" | "DELETE")
    })
}
