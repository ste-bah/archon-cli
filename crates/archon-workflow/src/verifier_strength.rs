//! Mechanical checks for verifier commands that cannot prove work.
//!
//! This module classifies only command shapes whose exit status is decidable
//! from the declaration itself. It deliberately does not judge whether a
//! falsifiable command proves the right business outcome.

use std::fmt;

mod prelude;
mod shell_lexer;

use shell_lexer::{Item, ListOp, Statement};

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
///
/// Only the script's final top-level command decides its exit status, so only
/// that command is judged (see [`script_verdict`]). A final compound command
/// (`if … fi`, a loop, a group, a function) and a script whose structure
/// cannot be parsed with certainty are undecidable here and yield `None`: the
/// dynamic can-fail probe decides those.
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
    let statements = shell_lexer::statements(&command)?;
    if let [only] = statements.as_slice()
        && !only.background
        && or_fallback(only, false, 0).is_none()
        && let Some(artifact_path) = own_artifact
            .map(normalize_path_token)
            .filter(|path| !path.is_empty())
        && existence_only_target(only)
            .is_some_and(|target| target == "{artifact_path}" || target == artifact_path)
    {
        return Some(VerifierStrengthDefect::OwnArtifactExistenceOnly { artifact_path });
    }
    Some(match script_verdict(&statements, 0)? {
        Verdict::Fallback(fallback) => VerifierStrengthDefect::FixedSuccessFallback { fallback },
        Verdict::Program(program) => VerifierStrengthDefect::FixedSuccessProgram { program },
    })
}

enum Verdict {
    Fallback(String),
    Program(String),
}

/// The final statement decides the status. A final command whose failure is
/// discarded (`… || true`, `… &`) is refused unless errexit already made an
/// earlier failing command end the script; a final command that always
/// succeeds after other statements is refused unless an earlier statement can
/// end the script with a failure.
fn script_verdict(statements: &[Statement], depth: usize) -> Option<Verdict> {
    let (last, earlier) = statements.split_last()?;
    let prelude = prelude::prelude(earlier);
    let discarded = if last.background {
        Some(format!("{} &", last.text))
    } else {
        or_fallback(last, prelude.pipefail, depth)
    };
    if let Some(label) = discarded {
        return (!prelude.errexit_failure).then_some(Verdict::Fallback(label));
    }
    let label = list_fixed_label(last, prelude.pipefail, depth)?;
    if earlier.is_empty() {
        return Some(Verdict::Program(label));
    }
    (!prelude.ends_script && !prelude.errexit_failure)
        .then(|| Verdict::Fallback(format!("; {}", last.text)))
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

/// Strip one pair of outer quotes, but only when they enclose the whole
/// command as a single quoted word: `'a' && b 'c'` starts and ends with a quote
/// without being one.
fn strip_balanced_outer(raw: &str) -> String {
    let raw = raw.trim();
    let (Some(first), Some(last)) = (raw.chars().next(), raw.chars().last()) else {
        return String::new();
    };
    if raw.len() < 2 || first != last || !matches!(first, '\'' | '"' | '`') {
        return raw.to_string();
    }
    let inner = &raw[1..raw.len() - 1];
    let encloses = if first == '\'' {
        !inner.contains('\'')
    } else {
        let mut escaped = false;
        let mut closed_early = false;
        for ch in inner.chars() {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == first {
                closed_early = true;
                break;
            }
        }
        !closed_early && !escaped
    };
    if encloses {
        inner.trim().to_string()
    } else {
        raw.to_string()
    }
}

/// The script of `bash -c '<script>'` / `sh -lc '<script>'`, when the wrapper
/// is the whole command and its script is a single word.
pub fn shell_wrapper_inner(command: &str) -> Option<String> {
    let command = command.trim();
    for shell in ["bash", "sh"] {
        let Some(rest) = command.strip_prefix(shell) else {
            continue;
        };
        if !rest.starts_with([' ', '\t']) {
            continue;
        }
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("-c").or_else(|| rest.strip_prefix("-lc")) else {
            continue;
        };
        if !rest.starts_with([' ', '\t']) {
            continue;
        }
        let script = rest.trim();
        let inner = strip_balanced_outer(script);
        return (inner != script || !script.contains(char::is_whitespace)).then_some(inner);
    }
    None
}

/// The label of a statement whose status is zero whatever its commands do.
fn statement_fixed_label(statement: &Statement, pipefail: bool, depth: usize) -> Option<String> {
    or_fallback(statement, pipefail, depth).or_else(|| list_fixed_label(statement, pipefail, depth))
}

/// `… || <always-succeeds>`: the list's status is zero whichever branch ran.
fn or_fallback(statement: &Statement, pipefail: bool, depth: usize) -> Option<String> {
    let last = statement.items.last()?;
    if statement.items.len() < 2 || last.op != Some(ListOp::Or) {
        return None;
    }
    item_fixed_label(last, pipefail, depth).map(|label| format!("|| {label}"))
}

/// Every pipeline of the list always succeeds.
fn list_fixed_label(statement: &Statement, pipefail: bool, depth: usize) -> Option<String> {
    let mut label = String::new();
    for item in &statement.items {
        let item_label = item_fixed_label(item, pipefail, depth)?;
        match item.op {
            Some(ListOp::And) => label.push_str(" && "),
            Some(ListOp::Or) => label.push_str(" || "),
            None => {}
        }
        label.push_str(&item_label);
    }
    (!label.is_empty()).then_some(label)
}

/// A pipeline's status is its last stage's, or under `pipefail` the first
/// failing stage's.
fn item_fixed_label(item: &Item, pipefail: bool, depth: usize) -> Option<String> {
    if pipefail {
        let labels = item
            .stages
            .iter()
            .map(|stage| stage_fixed_label(&stage.text, stage.compound, depth))
            .collect::<Option<Vec<_>>>()?;
        return Some(labels.join(" | "));
    }
    let last = item.stages.last()?;
    stage_fixed_label(&last.text, last.compound, depth)
}

fn stage_fixed_label(text: &str, compound: bool, depth: usize) -> Option<String> {
    if compound {
        return None;
    }
    let normalized = normalized_verifier_command(text);
    if normalized != text.trim() {
        return (depth < 4).then(|| script_fixed_label(&normalized, depth + 1))?;
    }
    let mut tokens = command_tokens(&normalized);
    while tokens
        .first()
        .is_some_and(|token| token == "command" || token == "builtin")
    {
        tokens.remove(0);
    }
    if let Some(program) = tokens.first_mut()
        && program.starts_with('/')
        && let Some(base) = program.rsplit('/').next().map(str::to_string)
        && is_fixed_success_program(&base)
    {
        *program = base;
    }
    fixed_success_single(&tokens).or_else(|| {
        tokens
            .first()
            .filter(|program| is_fixed_success_program(program))
            .cloned()
    })
}

fn script_fixed_label(script: &str, depth: usize) -> Option<String> {
    match script_verdict(&shell_lexer::statements(script)?, depth)? {
        Verdict::Fallback(label) | Verdict::Program(label) => Some(label),
    }
}

fn existence_only_target(statement: &Statement) -> Option<String> {
    let (first, rest) = statement.items.split_first()?;
    if statement.items.iter().any(|item| {
        item.op == Some(ListOp::Or) || item.stages.len() != 1 || item.stages[0].compound
    }) || rest
        .iter()
        .any(|item| item_fixed_label(item, false, 0).is_none())
    {
        return None;
    }
    let tokens = command_tokens(&first.stages[0].text);
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

fn fixed_success_single(tokens: &[String]) -> Option<String> {
    match tokens {
        [exit, code] if exit == "exit" && code == "0" => Some("exit 0".into()),
        // `wait` with no operand returns zero once every job has finished.
        [wait] if wait == "wait" => Some("wait".into()),
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
