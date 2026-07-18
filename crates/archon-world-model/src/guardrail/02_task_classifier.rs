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
    let Some(segments) = split_shell_chain(command) else {
        return RuntimeTaskClass::GeneralAnswer;
    };

    segments
        .iter()
        .map(|segment| classify_bash_segment(segment))
        .max_by_key(|class| class_priority(*class))
        .unwrap_or(RuntimeTaskClass::GeneralAnswer)
}

fn classify_bash_segment(command: &str) -> RuntimeTaskClass {
    let Some(tokens) = command_tokens(command) else {
        return RuntimeTaskClass::GeneralAnswer;
    };
    if let Some(inner) = shell_command(&tokens) {
        return classify_bash_command(&serde_json::json!({ "command": inner }));
    }
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

fn shell_command(tokens: &[String]) -> Option<&str> {
    let executable = tokens.first().map(|token| executable_name(token))?;
    if !matches!(executable.as_str(), "bash" | "sh") {
        return None;
    }
    let flag_count = tokens.iter().skip(1).take_while(|token| token.starts_with('-')).count();
    tokens
        .get(1..=flag_count)
        .filter(|flags| flags.iter().any(|flag| flag.contains('c')))
        .and_then(|_| tokens.get(flag_count + 1))
        .map(String::as_str)
}

fn class_priority(class: RuntimeTaskClass) -> u8 {
    match class {
        RuntimeTaskClass::DataMutation => 4,
        RuntimeTaskClass::ExternalSideEffect => 3,
        RuntimeTaskClass::VerificationOnly => 2,
        RuntimeTaskClass::CodingChange
        | RuntimeTaskClass::PipelineExecution
        | RuntimeTaskClass::ResearchAnswer
        | RuntimeTaskClass::Refactor
        | RuntimeTaskClass::Debugging => 1,
        RuntimeTaskClass::GeneralAnswer => 0,
    }
}

fn split_shell_chain(command: &str) -> Option<Vec<&str>> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let bytes = command.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' && quote != Some(b'\'') {
            escaped = true;
        } else if matches!(byte, b'\'' | b'"') {
            quote = match quote {
                Some(current) if current == byte => None,
                None => Some(byte),
                current => current,
            };
        } else if quote.is_none() && matches!(byte, b';' | b'\n' | b'|' | b'&') {
            push_segment(&mut segments, &command[start..index]);
            if matches!(byte, b'|' | b'&') && bytes.get(index + 1) == Some(&byte) {
                index += 1;
            }
            start = index + 1;
        }
        index += 1;
    }

    if quote.is_some() || escaped {
        return None;
    }
    push_segment(&mut segments, &command[start..]);
    Some(segments)
}

fn push_segment<'a>(segments: &mut Vec<&'a str>, segment: &'a str) {
    let segment = segment.trim();
    if !segment.is_empty() {
        segments.push(segment);
    }
}

fn command_tokens(command: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in command.chars() {
        if escaped {
            token.push(character);
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if matches!(character, '\'' | '"') {
            match quote {
                Some(current) if current == character => quote = None,
                None => quote = Some(character),
                Some(_) => token.push(character),
            }
        } else if character.is_whitespace() && quote.is_none() {
            push_token(&mut tokens, &mut token);
        } else {
            token.push(character);
        }
    }

    if quote.is_some() || escaped {
        return None;
    }
    push_token(&mut tokens, &mut token);
    Some(tokens)
}

fn push_token(tokens: &mut Vec<String>, token: &mut String) {
    if !token.is_empty() {
        tokens.push(std::mem::take(token));
    }
}

fn executable_name(token: &str) -> String {
    let basename = token.rsplit(['/', '\\']).next().unwrap_or(token);
    basename
        .to_ascii_lowercase()
        .strip_suffix(".exe")
        .unwrap_or(basename)
        .to_owned()
}

fn git_subcommand(tokens: &[String]) -> Option<&str> {
    let mut index = 1;
    while let Some(token) = tokens.get(index) {
        if !token.starts_with('-') {
            return Some(token);
        }
        index += 1;
        if matches!(token.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
            index += 1;
        }
    }
    None
}

fn is_verification_command(tokens: &[String]) -> bool {
    let Some(executable) = tokens.first().map(|token| executable_name(token)) else {
        return false;
    };
    let operation = tokens.get(1).map(String::as_str);
    matches!(
        (executable.as_str(), operation),
        ("cargo", Some("test" | "check" | "build" | "clippy" | "fmt")) | ("pytest", _)
    ) || matches!(
        (executable.as_str(), operation, tokens.get(2).map(String::as_str)),
        ("npm" | "pnpm" | "yarn" | "bun", Some("test"), _)
            | ("npm" | "pnpm" | "yarn" | "bun", Some("run"), Some("test" | "build" | "lint" | "typecheck"))
    )
}

fn is_external_command(tokens: &[String]) -> bool {
    let Some(executable) = tokens.first().map(|token| executable_name(token)) else {
        return false;
    };
    (executable == "git" && git_subcommand(tokens) == Some("push"))
        || matches!(executable.as_str(), "deploy" | "publish")
        || matches!(
            (executable.as_str(), tokens.get(1).map(String::as_str)),
            ("npm" | "pnpm" | "yarn" | "bun", Some("publish" | "deploy"))
        )
}

fn is_destructive_command(tokens: &[String]) -> bool {
    tokens
        .first()
        .is_some_and(|token| executable_name(token) == "rm")
        || has_executable_sql_mutation(tokens)
}

fn has_executable_sql_mutation(tokens: &[String]) -> bool {
    let Some((database_client, arguments)) = tokens.split_first() else {
        return false;
    };
    is_database_client(database_client)
        && sql_command_argument(arguments).is_some_and(has_sql_mutation)
}

fn is_database_client(command: &str) -> bool {
    matches!(
        executable_name(command).as_str(),
        "psql" | "sqlite3" | "mysql" | "mariadb" | "sqlcmd"
    )
}

fn sql_command_argument(arguments: &[String]) -> Option<&str> {
    arguments
        .windows(2)
        .find(|pair| matches!(pair[0].as_str(), "-c" | "--command" | "-e" | "-Q"))
        .map(|pair| pair[1].as_str())
}

fn has_sql_mutation(sql: &str) -> bool {
    let mut word = String::new();
    let mut in_literal = false;
    let mut characters = sql.chars().peekable();

    while let Some(character) = characters.next() {
        if character == '\'' {
            if in_literal && characters.peek() == Some(&'\'') {
                characters.next();
            } else {
                in_literal = !in_literal;
            }
        } else if !in_literal && character.is_ascii_alphabetic() {
            word.push(character.to_ascii_uppercase());
        } else if !in_literal && !word.is_empty() {
            if matches!(word.as_str(), "DROP" | "DELETE") {
                return true;
            }
            word.clear();
        }
    }

    !in_literal && matches!(word.as_str(), "DROP" | "DELETE")
}
