const MAX_SHELL_SUBSTITUTION_DEPTH: usize = 8;
const MAX_COMMAND_WRAPPER_DEPTH: usize = 8;

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
    input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map_or(RuntimeTaskClass::GeneralAnswer, |command| {
            classify_shell_command(command, 0)
        })
}

fn classify_shell_command(command: &str, depth: usize) -> RuntimeTaskClass {
    let Some(subcommands) = extract_shell_subcommands(command, depth) else {
        return RuntimeTaskClass::GeneralAnswer;
    };
    let Some(segments) = split_shell_chain(command) else {
        return RuntimeTaskClass::GeneralAnswer;
    };

    segments
        .iter()
        .map(|segment| classify_bash_segment(segment))
        .chain(
            subcommands
                .into_iter()
                .map(|subcommand| classify_shell_command(subcommand, depth + 1)),
        )
        .max_by_key(|class| class_priority(*class))
        .unwrap_or(RuntimeTaskClass::GeneralAnswer)
}

fn extract_shell_subcommands(command: &str, depth: usize) -> Option<Vec<&str>> {
    let mut subcommands = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let bytes = command.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' && quote != Some(b'\'') {
            escaped = true;
        } else if matches!(byte, b'\'' | b'"') {
            quote = toggle_quote(quote, byte);
        } else if quote != Some(b'\'') && matches!(byte, b'$' | b'`') {
            if depth >= MAX_SHELL_SUBSTITUTION_DEPTH {
                return None;
            }
            let (subcommand, next) = if byte == b'$' && bytes.get(index + 1) == Some(&b'(') {
                parenthesized_subcommand(command, index)?
            } else if byte == b'`' {
                backtick_subcommand(command, index)?
            } else {
                index += 1;
                continue;
            };
            subcommands.push(subcommand);
            index = next;
            continue;
        }
        index += 1;
    }

    if quote.is_some() || escaped {
        None
    } else {
        Some(subcommands)
    }
}

fn toggle_quote(quote: Option<u8>, byte: u8) -> Option<u8> {
    match quote {
        Some(current) if current == byte => None,
        None => Some(byte),
        current => current,
    }
}

fn parenthesized_subcommand(command: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = command.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    let mut nesting = 1;
    let mut index = start + 2;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' && quote != Some(b'\'') {
            escaped = true;
        } else if matches!(byte, b'\'' | b'"') {
            quote = toggle_quote(quote, byte);
        } else if quote.is_none() && byte == b'(' {
            nesting += 1;
        } else if quote.is_none() && byte == b')' {
            nesting -= 1;
            if nesting == 0 {
                return Some((&command[start + 2..index], index + 1));
            }
        }
        index += 1;
    }

    None
}

fn backtick_subcommand(command: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = command.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    let mut index = start + 1;

    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' && quote != Some(b'\'') {
            escaped = true;
        } else if matches!(byte, b'\'' | b'"') {
            quote = toggle_quote(quote, byte);
        } else if quote.is_none() && byte == b'`' {
            return Some((&command[start + 1..index], index + 1));
        }
        index += 1;
    }

    None
}

fn classify_bash_segment(command: &str) -> RuntimeTaskClass {
    let Some(tokens) = command_tokens(command) else {
        return RuntimeTaskClass::GeneralAnswer;
    };
    let Some(tokens) = normalize_command_wrappers(&tokens) else {
        return RuntimeTaskClass::GeneralAnswer;
    };
    if let Some(inner) = shell_command(tokens) {
        return classify_shell_command(inner, 0);
    }
    if is_destructive_command(tokens) {
        RuntimeTaskClass::DataMutation
    } else if is_external_command(tokens) {
        RuntimeTaskClass::ExternalSideEffect
    } else if is_verification_command(tokens) {
        RuntimeTaskClass::VerificationOnly
    } else {
        RuntimeTaskClass::GeneralAnswer
    }
}

fn normalize_command_wrappers(tokens: &[String]) -> Option<&[String]> {
    let mut command = tokens;
    for _ in 0..MAX_COMMAND_WRAPPER_DEPTH {
        let executable = command.first().map(|token| executable_name(token))?;
        command = match executable.as_str() {
            "env" => unwrap_env(command)?,
            "sudo" => unwrap_sudo(command)?,
            "command" => unwrap_command(command)?,
            _ => return Some(command),
        };
    }
    None
}

fn unwrap_env(tokens: &[String]) -> Option<&[String]> {
    let mut index = 1;
    while let Some(token) = tokens.get(index) {
        match token.as_str() {
            "--" => return tokens.get(index + 1..),
            "-i" | "--ignore-environment" => index += 1,
            "-u" | "--unset" | "-C" | "--chdir" => {
                tokens.get(index + 1)?;
                index += 2;
            }
            _ if token.starts_with("--unset=") || token.starts_with("--chdir=") => index += 1,
            _ if token.starts_with('-') => return None,
            _ if is_environment_assignment(token) => index += 1,
            _ => return tokens.get(index..),
        }
    }
    None
}

fn is_environment_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut characters = name.bytes();
    matches!(characters.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && characters.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn unwrap_sudo(tokens: &[String]) -> Option<&[String]> {
    let mut index = 1;
    while let Some(token) = tokens.get(index) {
        match token.as_str() {
            "--" => return tokens.get(index + 1..),
            "-b" | "-E" | "-e" | "-H" | "-K" | "-k" | "-n" | "-S" | "-s" | "-V" | "-v"
            | "--background" | "--preserve-env" | "--set-home" | "--reset-timestamp"
            | "--remove-timestamp" | "--non-interactive" | "--stdin" | "--shell" | "--version"
            | "--validate" => index += 1,
            "-u" | "--user" | "-g" | "--group" | "-h" | "--host" | "-C" | "--close-from" | "-r"
            | "--role" | "-t" | "--type" | "-T" | "--command-timeout" | "-R" | "--chroot"
            | "-D" | "--chdir" => {
                tokens.get(index + 1)?;
                index += 2;
            }
            _ if starts_sudo_option_with_value(token) => index += 1,
            _ if token.starts_with('-') => return None,
            _ => return tokens.get(index..),
        }
    }
    None
}

fn starts_sudo_option_with_value(token: &str) -> bool {
    [
        "--user=",
        "--group=",
        "--host=",
        "--close-from=",
        "--role=",
        "--type=",
        "--command-timeout=",
        "--chroot=",
        "--chdir=",
    ]
    .iter()
    .any(|prefix| token.starts_with(prefix))
        || matches!(
            token.as_bytes().first(),
            Some(b'u' | b'g' | b'h' | b'C' | b'r' | b't' | b'T' | b'R' | b'D')
        ) && token.starts_with('-')
            && token.len() > 2
}

fn unwrap_command(tokens: &[String]) -> Option<&[String]> {
    match tokens.get(1).map(String::as_str) {
        Some("--") => tokens.get(2..),
        Some("-p") => tokens.get(2..),
        Some(option) if option.starts_with('-') => None,
        Some(_) => tokens.get(1..),
        None => None,
    }
}

fn shell_command(tokens: &[String]) -> Option<&str> {
    let executable = tokens.first().map(|token| executable_name(token))?;
    if !matches!(executable.as_str(), "bash" | "sh") {
        return None;
    }
    let flag_count = tokens
        .iter()
        .skip(1)
        .take_while(|token| token.starts_with('-'))
        .count();
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
            quote = toggle_quote(quote, byte);
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
