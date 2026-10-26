use std::collections::BTreeMap;
use std::path::Path;

use std::path::PathBuf;
#[path = "workflow_acceptance_executability_mutation_paths_variables.rs"]
mod variables;
use variables::{VariableFlow, apply_assignments, join_variables, valid_variable};
#[path = "workflow_acceptance_executability_mutation_paths_fs.rs"]
mod fs_paths;
pub(super) use fs_paths::{diagnostic_path, normalize_absolute, relative_path};

#[path = "workflow_acceptance_executability_mutation_paths_lex.rs"]
mod lex;
pub(super) use lex::{group_has_exit, lex};

pub(super) const SUBSHELL_OPEN: &str = "\u{0}subshell-open";
pub(super) const SUBSHELL_CLOSE: &str = "\u{0}subshell-close";
const GROUP_OPEN: &str = "\u{0}group-open";
const GROUP_CLOSE: &str = "\u{0}group-close";

fn normalize(token: &str) -> &str {
    token.trim_start_matches("./").trim_end_matches('/')
}

pub(super) fn path_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !archon_workflow::acceptance_sibling_paths::is_path_character(c))
        .filter(|token| !token.ends_with('/'))
        .map(normalize)
        .filter(|token| {
            !token.is_empty()
                && !token.starts_with('-')
                && !token.starts_with('/')
                && token.len() <= 4096
                && Path::new(token)
                    .components()
                    .enumerate()
                    .all(|(index, part)| match part {
                        std::path::Component::Normal(name) => name != ".git" && name != "target",
                        std::path::Component::ParentDir => index == 0,
                        std::path::Component::CurDir => index == 0,
                        _ => false,
                    })
        })
        .map(str::to_string)
        .collect()
}

pub(super) fn effective_directories(
    commands: &[Vec<String>],
    initial: &Path,
) -> (Vec<Vec<String>>, Vec<Option<PathBuf>>, bool) {
    let mut directory = Some(initial.to_path_buf());
    let mut variables = BTreeMap::<String, String>::new();
    let mut previous_directory: Option<PathBuf> = None;
    let mut directory_stack: Vec<(Option<PathBuf>, Option<PathBuf>)> = Vec::new();
    let mut subshell_stack: Vec<(Option<PathBuf>, Option<PathBuf>, BTreeMap<String, String>)> =
        Vec::new();
    let mut flow_stack: Vec<VariableFlow> = Vec::new();
    let mut bases = Vec::with_capacity(commands.len());
    let mut expanded_commands = Vec::with_capacity(commands.len());
    let mut conditional_variables = std::collections::BTreeSet::<String>::new();
    let mut unknown_context = false;
    let mut conditional_list_changed = false;
    let mut conditional_list_exits = false;
    let mut condition_changed_directory = false;
    let mut group_depth = 0usize;
    let mut conditional_exit_directory: Option<PathBuf> = None;
    for (command_no, words) in commands.iter().enumerate() {
        if words.is_empty() {
            bases.push(directory.clone());
            expanded_commands.push(Vec::new());
            continue;
        }
        let connector = words.first().map(String::as_str).unwrap_or("");
        let conditional_continuation = matches!(connector, "\0and" | "\0or");
        if words.iter().any(|word| word == GROUP_OPEN) {
            bases.push(if connector == "\0or" && conditional_list_changed {
                None
            } else {
                directory.clone()
            });
            if connector == "\0or" && conditional_list_changed {
                conditional_list_exits = group_has_exit(commands, command_no);
                if conditional_list_exits {
                    conditional_exit_directory = directory.clone();
                }
                directory = None;
                previous_directory = None;
            }
            group_depth += 1;
            expanded_commands.push(words.clone());
            continue;
        }
        if words.iter().any(|word| word == GROUP_CLOSE) {
            group_depth = group_depth.saturating_sub(1);
            if group_depth == 0 && conditional_list_exits {
                directory = conditional_exit_directory.take();
                previous_directory = None;
            }
            bases.push(directory.clone());
            expanded_commands.push(words.clone());
            continue;
        }
        if conditional_continuation && conditional_list_changed {
            let command = words
                .iter()
                .find(|word| !word.starts_with('\0'))
                .map(String::as_str)
                .unwrap_or("");
            if connector == "\0or" && matches!(command, "exit" | "return") {
                conditional_list_exits = true;
            }
        }
        if !conditional_continuation && conditional_list_changed && group_depth == 0 {
            // A short-circuit list can stop at any command. If a directory
            // change was part of that list, both the changed and skipped
            // paths reach the next command.
            if !conditional_list_exits {
                directory = None;
                previous_directory = None;
                unknown_context = true;
            }
            conditional_list_changed = false;
            conditional_list_exits = false;
        }
        if !conditional_continuation && !conditional_variables.is_empty() {
            for name in std::mem::take(&mut conditional_variables) {
                variables.remove(&name);
            }
        }
        let words = words
            .iter()
            .filter(|word| !matches!(word.as_str(), "\0and" | "\0or" | GROUP_OPEN | GROUP_CLOSE))
            .cloned()
            .collect::<Vec<_>>();
        let words = words
            .iter()
            .map(|word| lex::substitute_leading_variables(word, &variables))
            .collect::<Vec<_>>();
        if words.iter().any(|word| word == SUBSHELL_OPEN) {
            subshell_stack.push((
                directory.clone(),
                previous_directory.clone(),
                variables.clone(),
            ));
        }
        bases.push(directory.clone());
        expanded_commands.push(words.clone());
        if words.iter().any(|word| word == SUBSHELL_CLOSE) {
            if let Some((saved, previous, saved_variables)) = subshell_stack.pop() {
                directory = saved;
                previous_directory = previous;
                variables = saved_variables;
            }
            continue;
        }
        let first = words.first().map(String::as_str).unwrap_or("");
        if first == "if" {
            flow_stack.push(VariableFlow::Branch {
                start: directory.clone(),
                previous: previous_directory.clone(),
                variables: variables.clone(),
                variable_ends: Vec::new(),
                ends: Vec::new(),
                previous_ends: Vec::new(),
                has_else: false,
            });
        } else if matches!(first, "while" | "until" | "for" | "select") {
            flow_stack.push(VariableFlow::Loop {
                start: directory.clone(),
                previous: previous_directory.clone(),
                changed: false,
                variables: variables.clone(),
                assigned: std::collections::BTreeSet::new(),
            });
        } else if first == "do"
            && !flow_stack
                .iter()
                .any(|flow| matches!(flow, VariableFlow::Loop { .. }))
        {
            flow_stack.push(VariableFlow::Loop {
                start: directory.clone(),
                previous: previous_directory.clone(),
                changed: false,
                variables: variables.clone(),
                assigned: std::collections::BTreeSet::new(),
            });
        }
        if matches!(first, "then" | "do") && condition_changed_directory {
            directory = None;
            previous_directory = None;
            if let Some(base) = bases.last_mut() {
                *base = None;
            }
            if let Some(flow) = flow_stack.last_mut() {
                match flow {
                    VariableFlow::Branch { start, .. } | VariableFlow::Loop { start, .. } => {
                        *start = None
                    }
                }
            }
            unknown_context = true;
            condition_changed_directory = false;
        }
        if matches!(first, "else" | "elif")
            && let Some(VariableFlow::Branch {
                start,
                variables: branch_variables,
                variable_ends,
                ends,
                previous_ends,
                has_else,
                ..
            }) = flow_stack.last_mut()
        {
            variable_ends.push(variables.clone());
            ends.push(directory.clone());
            previous_ends.push(previous_directory.clone());
            *has_else |= first == "else";
            directory = start.clone();
            previous_directory = None;
            variables = branch_variables.clone();
            if let Some(base) = bases.last_mut() {
                *base = directory.clone();
            }
        }
        if matches!(first, "fi" | "done" | "}") {
            if let Some(flow) = flow_stack.pop() {
                match flow {
                    VariableFlow::Branch {
                        start,
                        previous,
                        variables: branch_variables,
                        mut variable_ends,
                        mut ends,
                        mut previous_ends,
                        has_else,
                    } if first == "fi" => {
                        ends.push(directory.clone());
                        previous_ends.push(previous_directory.clone());
                        if !has_else {
                            ends.push(start.clone());
                            previous_ends.push(previous.clone());
                            variable_ends.push(branch_variables.clone());
                        }
                        variable_ends.push(variables.clone());
                        variables = join_variables(&variable_ends);
                        directory = ends
                            .first()
                            .cloned()
                            .filter(|value| value.is_some() && ends.iter().all(|end| end == value))
                            .flatten();
                        previous_directory = previous_ends
                            .first()
                            .cloned()
                            .filter(|value| previous_ends.iter().all(|end| end == value))
                            .flatten();
                        if directory.is_none() {
                            unknown_context = true;
                        }
                    }
                    VariableFlow::Loop {
                        start,
                        previous,
                        changed,
                        variables: loop_variables,
                        assigned,
                    } if first == "done" => {
                        variables = loop_variables;
                        for name in assigned {
                            variables.remove(&name);
                        }
                        if changed {
                            directory = None;
                            previous_directory = None;
                            unknown_context = true;
                        } else {
                            directory = start;
                            previous_directory = previous;
                        }
                    }
                    other => flow_stack.push(other),
                }
            }
            continue;
        }
        let command_index = words
            .iter()
            .position(|word| {
                !word.starts_with('\0')
                    && !matches!(
                        word.as_str(),
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "do"
                            | "while"
                            | "until"
                            | "!"
                            | "{"
                            | "("
                    )
            })
            .unwrap_or(0);
        if words.get(command_index).is_some_and(|word| word == "unset") {
            for name in words.iter().skip(command_index + 1) {
                if valid_variable(name) {
                    variables.remove(name);
                    for flow in &mut flow_stack {
                        if let VariableFlow::Loop { assigned, .. } = flow {
                            assigned.insert(name.clone());
                        }
                    }
                    if conditional_continuation {
                        conditional_variables.insert(name.clone());
                    }
                }
            }
        }
        let conditional_assignment = conditional_continuation
            || matches!(
                words.first().map(String::as_str),
                Some("if" | "while" | "until")
            );
        apply_assignments(
            &words,
            &mut variables,
            &mut flow_stack,
            &mut conditional_variables,
            conditional_assignment,
        );
        let Some(base) = directory.clone() else {
            continue;
        };
        let Some(program) = words
            .get(command_index)
            .and_then(|word| Path::new(word).file_name())
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if matches!(program.as_str(), "cd" | "pushd" | "popd")
            && matches!(first, "if" | "elif" | "while" | "until")
        {
            condition_changed_directory = true;
        }
        match program.as_str() {
            "cd" | "pushd" => {
                let operand = words
                    .iter()
                    .skip(command_index + 1)
                    .find(|word| word.as_str() == "-" || !word.starts_with('-'));
                let target = match operand.map(String::as_str) {
                    Some("-") if program == "cd" => previous_directory.clone(),
                    Some(path) if !path.contains('$') && !path.starts_with('`') => {
                        let path = Path::new(path);
                        Some(normalize_absolute(
                            if path.is_absolute() {
                                path.to_path_buf()
                            } else {
                                base.join(path)
                            }
                            .as_path(),
                        ))
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    if target.is_dir() {
                        if flow_stack
                            .iter()
                            .any(|flow| matches!(flow, VariableFlow::Loop { .. }))
                            && !matches!(first, "while" | "until")
                        {
                            // A loop body may run zero or many times. The
                            // directory after its conditional cd is not a
                            // single known path at any later body point.
                            for flow in &mut flow_stack {
                                if let VariableFlow::Loop { changed, .. } = flow {
                                    *changed = true;
                                }
                            }
                            directory = None;
                            previous_directory = None;
                            unknown_context = true;
                            continue;
                        }
                        if conditional_continuation
                            || commands
                                .get(command_no + 1)
                                .and_then(|words| words.first())
                                .is_some_and(|word| matches!(word.as_str(), "\0and" | "\0or"))
                        {
                            conditional_list_changed = true;
                        }
                        for flow in &mut flow_stack {
                            if let VariableFlow::Loop { changed, .. } = flow {
                                *changed = true;
                            }
                        }
                        if program == "pushd" {
                            directory_stack.push((Some(base.clone()), previous_directory.clone()));
                        }
                        previous_directory = Some(base);
                        directory = Some(target);
                    }
                } else {
                    for flow in &mut flow_stack {
                        if let VariableFlow::Loop { changed, .. } = flow {
                            *changed = true;
                        }
                    }
                    directory = None;
                    unknown_context = true;
                }
            }
            "popd" => {
                if flow_stack
                    .iter()
                    .any(|flow| matches!(flow, VariableFlow::Loop { .. }))
                {
                    for flow in &mut flow_stack {
                        if let VariableFlow::Loop { changed, .. } = flow {
                            *changed = true;
                        }
                    }
                    directory = None;
                    previous_directory = None;
                    unknown_context = true;
                } else if let Some((saved, previous)) = directory_stack.pop() {
                    directory = saved;
                    previous_directory = previous;
                }
            }
            _ => {}
        }
    }
    (expanded_commands, bases, unknown_context)
}

/// Apply one-command `-C`/`--directory` scopes for common shell tools.
pub(super) fn command_directories(
    commands: &[Vec<String>],
    mut bases: Vec<Option<PathBuf>>,
) -> Vec<Option<PathBuf>> {
    for (index, words) in commands.iter().enumerate() {
        let command_index = words
            .iter()
            .position(|word| {
                !word.starts_with('\0')
                    && !matches!(
                        word.as_str(),
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "do"
                            | "while"
                            | "until"
                            | "!"
                            | "{"
                            | "("
                    )
            })
            .unwrap_or(0);
        let program = words
            .get(command_index)
            .and_then(|word| Path::new(word).file_name())
            .map(|name| name.to_string_lossy().into_owned());
        if !matches!(program.as_deref(), Some("env" | "git" | "make")) {
            continue;
        }
        let args = &words[command_index + 1..];
        let directory = args.iter().enumerate().find_map(|(i, word)| {
            if matches!(word.as_str(), "-C" | "--directory") {
                args.get(i + 1).cloned()
            } else {
                word.strip_prefix("--directory=").map(str::to_string)
            }
        });
        if let (Some(base), Some(directory)) = (bases.get(index).cloned().flatten(), directory) {
            let path = Path::new(&directory);
            bases[index] = Some(normalize_absolute(
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    base.join(path)
                }
                .as_path(),
            ));
        }
    }
    bases
}
