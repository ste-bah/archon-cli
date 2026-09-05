//! Mechanical checks for verifier commands that cannot prove work.
//!
//! This module classifies only command shapes whose exit status is decidable
//! from the declaration itself. It deliberately does not judge whether a
//! falsifiable command proves the right business outcome.

use std::fmt;

use crate::task_universe::WorkflowV2DeliverableContract;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifierStrengthDefect {
    MissingExecutionObligation,
    OwnArtifactExistenceOnly { artifact_path: String },
    FixedSuccessProgram { program: String },
    FixedSuccessFallback { fallback: String },
}

impl fmt::Display for VerifierStrengthDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let replacement = "replace it with a command that can exit non-zero when the declared outcome is false, or declare a real positive instance floor/source binding; deleting the verifier does not satisfy this contract because it also removes the task's execution obligation";
        match self {
            Self::MissingExecutionObligation => write!(
                f,
                "deliverable contract has neither a verifier nor a positive instance obligation; {replacement}"
            ),
            Self::OwnArtifactExistenceOnly { artifact_path } => write!(
                f,
                "verifier only tests whether its own artifact_path '{artifact_path}' exists, which the generated deliverable gate already checks for existence and non-emptiness; {replacement}"
            ),
            Self::FixedSuccessProgram { program } => write!(
                f,
                "verifier's effective program '{program}' reports data but does not judge it, so its exit status cannot prove the outcome; {replacement}"
            ),
            Self::FixedSuccessFallback { fallback } => write!(
                f,
                "verifier ends with '{fallback}', which converts failure into success; {replacement}"
            ),
        }
    }
}

/// A deterministic defect in a verifier declaration, if one exists.
///
/// `own_artifact` is the contract path whose existence/non-emptiness the host
/// already verifies. `contract` supplies the alternative positive instance
/// obligation when no command is declared.
pub fn verifier_strength_defect(
    command: Option<&str>,
    own_artifact: Option<&str>,
    contract: Option<&WorkflowV2DeliverableContract>,
) -> Option<VerifierStrengthDefect> {
    let command = command.map(str::trim).filter(|value| !value.is_empty());
    let Some(command) = command else {
        return (!contract.is_some_and(has_positive_instance_obligation))
            .then_some(VerifierStrengthDefect::MissingExecutionObligation);
    };
    let command = normalized_verifier_command(command);

    if let Some(fallback) = fixed_success_fallback(&command) {
        return Some(VerifierStrengthDefect::FixedSuccessFallback { fallback });
    }
    if let Some(artifact_path) = own_artifact
        .map(normalize_path_token)
        .filter(|path| !path.is_empty())
        && existence_only_target(&command)
            .is_some_and(|target| target == "{artifact_path}" || target == artifact_path)
    {
        return Some(VerifierStrengthDefect::OwnArtifactExistenceOnly { artifact_path });
    }
    if let Some(program) = fixed_success_command(&command) {
        return Some(VerifierStrengthDefect::FixedSuccessProgram { program });
    }
    let program = effective_program(&command)?;
    is_fixed_success_program(&program)
        .then_some(VerifierStrengthDefect::FixedSuccessProgram { program })
}

/// Acceptance must exercise the deliverable; an implementation-owned instance
/// inventory is evidence of presence, not independent evidence of execution.
pub fn acceptance_verifier_strength_defect(
    command: Option<&str>,
    own_artifact: Option<&str>,
) -> Option<VerifierStrengthDefect> {
    verifier_strength_defect(command, own_artifact, None)
}

fn has_positive_instance_obligation(contract: &WorkflowV2DeliverableContract) -> bool {
    contract.min_instances >= 1
        || (contract.instance_artifact_field.is_some()
            && (contract.instance_source_path.is_some() || contract.registry_path.is_some())
            && (contract.instance_source_records_field.is_some()
                || contract.registry_records_field.is_some()))
}

pub fn normalized_verifier_command(raw: &str) -> String {
    let mut command = raw.trim().to_string();
    for _ in 0..4 {
        let stripped = strip_balanced_outer(&command);
        if stripped != command {
            command = stripped;
            continue;
        }
        let Some(inner) = shell_wrapper_inner(&command) else {
            break;
        };
        command = inner;
    }
    command
}

fn strip_balanced_outer(raw: &str) -> String {
    let raw = raw.trim();
    let Some(first) = raw.chars().next() else {
        return String::new();
    };
    let last = raw.chars().last().unwrap_or(first);
    if raw.len() >= 2 && matches!((first, last), ('\'', '\'') | ('"', '"') | ('`', '`')) {
        raw[first.len_utf8()..raw.len() - last.len_utf8()]
            .trim()
            .to_string()
    } else {
        raw.to_string()
    }
}

pub fn shell_wrapper_inner(command: &str) -> Option<String> {
    let command = command.trim();
    for shell in ["bash", "sh"] {
        let Some(rest) = command.strip_prefix(shell) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("-c").or_else(|| rest.strip_prefix("-lc")) else {
            continue;
        };
        return Some(strip_balanced_outer(rest.trim_start()));
    }
    None
}

fn fixed_success_fallback(command: &str) -> Option<String> {
    if let Some(tail) = top_level_tail(command, "||") {
        let tail = normalized_verifier_command(tail.trim());
        if matches!(tail.as_str(), "true" | ":") {
            return Some(format!("|| {tail}"));
        }
    }
    let tail = top_level_segments(command, ";")
        .into_iter()
        .rev()
        .find(|segment| !segment.trim().is_empty())?;
    if tail.trim() == command.trim() {
        return None;
    }
    let tail = normalized_verifier_command(tail);
    fixed_success_segment(&tail).map(|_| format!("; {tail}"))
}

fn top_level_tail<'a>(command: &'a str, operator: &str) -> Option<&'a str> {
    let bytes = command.as_bytes();
    let op = operator.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    let mut found = None;
    let mut index = 0;
    while index + op.len() <= bytes.len() {
        let ch = bytes[index] as char;
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            index += 1;
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
            index += 1;
            continue;
        }
        if quote.is_none() && &bytes[index..index + op.len()] == op {
            found = Some(index + op.len());
            index += op.len();
            continue;
        }
        index += 1;
    }
    found.map(|offset| &command[offset..])
}

fn existence_only_target(command: &str) -> Option<String> {
    if command.contains("||") || command.contains('|') {
        return None;
    }
    let semicolon_segments = top_level_segments(command, ";")
        .into_iter()
        .filter(|segment| !segment.trim().is_empty())
        .collect::<Vec<_>>();
    let command = match semicolon_segments.as_slice() {
        [single] => *single,
        _ => return None,
    };
    let segments = top_level_segments(command, "&&");
    let (first, tail) = segments.split_first()?;
    if tail.iter().any(|segment| {
        effective_program(segment).is_none_or(|program| !is_fixed_success_program(&program))
    }) {
        return None;
    }
    let tokens = command_tokens(first);
    match tokens.as_slice() {
        [test, flag, target] if test == "test" && unary_file_flag(flag) => {
            Some(normalize_path_token(target))
        }
        [open, flag, target, close] if open == "[" && close == "]" && unary_file_flag(flag) => {
            Some(normalize_path_token(target))
        }
        [program, operands @ ..] if matches!(program.as_str(), "ls" | "stat") => {
            let paths: Vec<_> = operands
                .iter()
                .filter(|token| !token.starts_with('-'))
                .collect();
            (paths.len() == 1).then(|| normalize_path_token(paths[0]))
        }
        _ => None,
    }
}

fn unary_file_flag(flag: &str) -> bool {
    matches!(flag, "-e" | "-f" | "-s" | "-d" | "-r" | "-w" | "-x" | "-L")
}

fn top_level_segments<'a>(command: &'a str, operator: &str) -> Vec<&'a str> {
    let bytes = command.as_bytes();
    let op = operator.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0;
    let mut segments = Vec::new();
    let mut index = 0;
    while index + op.len() <= bytes.len() {
        let ch = bytes[index] as char;
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            index += 1;
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
            index += 1;
            continue;
        }
        if quote.is_none() && &bytes[index..index + op.len()] == op {
            segments.push(command[start..index].trim());
            index += op.len();
            start = index;
            continue;
        }
        index += 1;
    }
    segments.push(command[start..].trim());
    segments
}

fn effective_program(command: &str) -> Option<String> {
    let tail = top_level_tail(command, "|").unwrap_or(command).trim();
    let tail = normalized_verifier_command(tail);
    command_tokens(&tail).first().cloned()
}

fn command_tokens(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|token| token.trim_matches(['\'', '"', '`']).to_string())
        .filter(|token| !token.is_empty())
        .collect()
}

fn normalize_path_token(path: &str) -> String {
    let normalized = path
        .trim()
        .trim_matches(['\'', '"', '`'])
        .replace('\\', "/");
    if normalized == "{artifact_path}" {
        return normalized;
    }
    normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .trim_end_matches('/')
        .to_string()
}

fn fixed_success_command(command: &str) -> Option<String> {
    let segments = top_level_segments(command, "&&");
    if segments.len() > 1 {
        let labels = segments
            .iter()
            .map(|segment| fixed_success_segment(segment))
            .collect::<Option<Vec<_>>>()?;
        return Some(labels.join(" && "));
    }
    fixed_success_single(command)
}

fn fixed_success_segment(command: &str) -> Option<String> {
    fixed_success_single(command).or_else(|| {
        let program = effective_program(command)?;
        is_fixed_success_program(&program).then_some(program)
    })
}

fn fixed_success_single(command: &str) -> Option<String> {
    let tokens = command_tokens(command);
    match tokens.as_slice() {
        [exit, code] if exit == "exit" && code == "0" => Some("exit 0".into()),
        [program, flag]
            if flag == "--version"
                || flag == "version"
                || flag == "-V"
                || flag.starts_with("-Vv") =>
        {
            Some(format!("{program} {flag}"))
        }
        [program, flag, script @ ..]
            if matches!(program.as_str(), "python" | "python3" | "node")
                && flag == "-c"
                && !script.is_empty()
                && script.iter().all(|part| {
                    let part = part.trim();
                    part.starts_with("print(")
                        || part.starts_with("console.log(")
                        || part.starts_with("process.stdout.write(")
                }) =>
        {
            Some(format!("{program} -c print-only"))
        }
        _ => None,
    }
}

fn is_fixed_success_program(program: &str) -> bool {
    matches!(
        program,
        "true" | ":" | "echo" | "printf" | "yes" | "wc" | "head" | "tail" | "cat" | "sort"
    )
}
