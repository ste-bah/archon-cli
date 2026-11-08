use std::collections::BTreeMap;
use std::path::Path;

use std::path::PathBuf;
#[path = "workflow_acceptance_executability_mutation_paths_variables.rs"]
mod variables;
use variables::{VariableFlow, apply_assignments, join_variables, valid_variable};
#[path = "workflow_acceptance_executability_mutation_paths_scope.rs"]
mod scope;
use scope::ScopeState;
#[path = "workflow_acceptance_executability_mutation_paths_fs.rs"]
mod fs_paths;
use fs_paths::normalize;
pub(super) use fs_paths::{
    diagnostic_path, normalize_absolute, relative_path, whole_tree_path, whole_tree_target,
};

#[path = "workflow_acceptance_executability_mutation_paths_lex.rs"]
mod lex;
#[cfg(test)]
pub(super) use lex::lex;
pub(super) use lex::{group_has_exit, lex_with_quotes};

pub(super) const SUBSHELL_OPEN: &str = "\u{0}subshell-open";
pub(super) const SUBSHELL_CLOSE: &str = "\u{0}subshell-close";
const GROUP_OPEN: &str = "\u{0}group-open";
const GROUP_CLOSE: &str = "\u{0}group-close";

pub(super) fn path_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !archon_workflow::acceptance_sibling_paths::is_path_character(c))
        .filter(|token| !token.ends_with('/') || whole_tree_path(token))
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

/// Absolute observations are separate from the mutation token grammar.
pub(super) fn diagnostic_path_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| {
        !(archon_workflow::acceptance_sibling_paths::is_path_character(c)
            || matches!(c, ':' | '\\'))
    })
    .filter(|token| token.len() <= 4096 && Path::new(token).has_root())
    .map(str::to_owned)
    .collect()
}

pub(super) fn effective_directories(
    commands: &[Vec<String>],
    initial: &Path,
) -> (Vec<Vec<String>>, Vec<Option<PathBuf>>, bool) {
    let mut state = ScopeState::default();
    state.directory = Some(initial.to_path_buf());
    let mut subshell_stack = Vec::new();
    let mut bases = Vec::with_capacity(commands.len());
    let mut expanded_commands = Vec::with_capacity(commands.len());
    let mut unknown_context = false;
    for (command_no, words) in commands.iter().enumerate() {
        if words.is_empty() {
            bases.push(state.directory.clone());
            expanded_commands.push(Vec::new());
            continue;
        }
        let connector = words.first().map(String::as_str).unwrap_or("");
        let conditional_continuation = matches!(connector, "\0and" | "\0or");
        if words.iter().any(|word| word == GROUP_OPEN) {
            bases.push(if connector == "\0or" && state.conditional_list_changed {
                None
            } else {
                state.directory.clone()
            });
            if connector == "\0or" && state.conditional_list_changed {
                state.conditional_list_exits = group_has_exit(commands, command_no);
                if state.conditional_list_exits {
                    state.conditional_exit_directory = state.directory.clone();
                }
                state.directory = None;
                state.previous_directory = None;
            }
            state.group_depth += 1;
            expanded_commands.push(words.clone());
            continue;
        }
        if words.iter().any(|word| word == GROUP_CLOSE) {
            state.group_depth = state.group_depth.saturating_sub(1);
            if state.group_depth == 0 && state.conditional_list_exits {
                state.directory = state.conditional_exit_directory.take();
                state.previous_directory = None;
            }
            bases.push(state.directory.clone());
            expanded_commands.push(words.clone());
            continue;
        }
        if conditional_continuation && state.conditional_list_changed {
            let command = words
                .iter()
                .find(|word| !word.starts_with('\0'))
                .map(String::as_str)
                .unwrap_or("");
            if connector == "\0or" && matches!(command, "exit" | "return") {
                state.conditional_list_exits = true;
            }
        }
        if !conditional_continuation && state.conditional_list_changed && state.group_depth == 0 {
            // A short-circuit list can stop at any command. If a directory
            // change was part of that list, both the changed and skipped
            // paths reach the next command.
            if !state.conditional_list_exits {
                state.directory = None;
                state.previous_directory = None;
                unknown_context = true;
            }
            state.conditional_list_changed = false;
            state.conditional_list_exits = false;
        }
        if !conditional_continuation && !state.conditional_variables.is_empty() {
            for name in std::mem::take(&mut state.conditional_variables) {
                state.variables.remove(&name);
            }
        }
        let words = words
            .iter()
            .filter(|word| !matches!(word.as_str(), "\0and" | "\0or" | GROUP_OPEN | GROUP_CLOSE))
            .cloned()
            .collect::<Vec<_>>();
        let words = words
            .iter()
            .map(|word| lex::substitute_leading_variables(word, &state.variables))
            .collect::<Vec<_>>();
        if words.iter().any(|word| word == SUBSHELL_OPEN) {
            subshell_stack.push(state.clone());
            state = state.child();
        }
        bases.push(state.directory.clone());
        expanded_commands.push(words.clone());
        if words.iter().any(|word| word == SUBSHELL_CLOSE) {
            if let Some(saved) = subshell_stack.pop() {
                state = saved;
            }
            continue;
        }
        let first = words.first().map(String::as_str).unwrap_or("");
        if first == "if" {
            state.flow_stack.push(VariableFlow::Branch {
                start: state.directory.clone(),
                previous: state.previous_directory.clone(),
                variables: state.variables.clone(),
                variable_ends: Vec::new(),
                ends: Vec::new(),
                previous_ends: Vec::new(),
                has_else: false,
            });
        } else if matches!(first, "while" | "until" | "for" | "select") {
            state.flow_stack.push(VariableFlow::Loop {
                start: state.directory.clone(),
                previous: state.previous_directory.clone(),
                changed: false,
                variables: state.variables.clone(),
                assigned: std::collections::BTreeSet::new(),
            });
        } else if first == "do"
            && !state
                .flow_stack
                .iter()
                .any(|flow| matches!(flow, VariableFlow::Loop { .. }))
        {
            state.flow_stack.push(VariableFlow::Loop {
                start: state.directory.clone(),
                previous: state.previous_directory.clone(),
                changed: false,
                variables: state.variables.clone(),
                assigned: std::collections::BTreeSet::new(),
            });
        }
        if matches!(first, "then" | "do") && state.condition_changed_directory {
            state.directory = None;
            state.previous_directory = None;
            if let Some(base) = bases.last_mut() {
                *base = None;
            }
            if let Some(flow) = state.flow_stack.last_mut() {
                match flow {
                    VariableFlow::Branch { start, .. } | VariableFlow::Loop { start, .. } => {
                        *start = None
                    }
                }
            }
            unknown_context = true;
            state.condition_changed_directory = false;
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
            }) = state.flow_stack.last_mut()
        {
            variable_ends.push(state.variables.clone());
            ends.push(state.directory.clone());
            previous_ends.push(state.previous_directory.clone());
            *has_else |= first == "else";
            state.directory = start.clone();
            state.previous_directory = None;
            state.variables = branch_variables.clone();
            if let Some(base) = bases.last_mut() {
                *base = state.directory.clone();
            }
        }
        if matches!(first, "fi" | "done" | "}") {
            if let Some(flow) = state.flow_stack.pop() {
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
                        ends.push(state.directory.clone());
                        previous_ends.push(state.previous_directory.clone());
                        if !has_else {
                            ends.push(start.clone());
                            previous_ends.push(previous.clone());
                            variable_ends.push(branch_variables.clone());
                        }
                        variable_ends.push(state.variables.clone());
                        state.variables = join_variables(&variable_ends);
                        state.directory = ends
                            .first()
                            .cloned()
                            .filter(|value| value.is_some() && ends.iter().all(|end| end == value))
                            .flatten();
                        state.previous_directory = previous_ends
                            .first()
                            .cloned()
                            .filter(|value| previous_ends.iter().all(|end| end == value))
                            .flatten();
                        if state.directory.is_none() {
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
                        state.variables = loop_variables;
                        for name in assigned {
                            state.variables.remove(&name);
                        }
                        if changed {
                            state.directory = None;
                            state.previous_directory = None;
                            unknown_context = true;
                        } else {
                            state.directory = start;
                            state.previous_directory = previous;
                        }
                    }
                    other => state.flow_stack.push(other),
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
                    state.variables.remove(name);
                    for flow in &mut state.flow_stack {
                        if let VariableFlow::Loop { assigned, .. } = flow {
                            assigned.insert(name.clone());
                        }
                    }
                    if conditional_continuation {
                        state.conditional_variables.insert(name.clone());
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
            &mut state.variables,
            &mut state.flow_stack,
            &mut state.conditional_variables,
            conditional_assignment,
        );
        let Some(base) = state.directory.clone() else {
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
            state.condition_changed_directory = true;
        }
        match program.as_str() {
            "cd" | "pushd" => {
                let operand = words
                    .iter()
                    .skip(command_index + 1)
                    .find(|word| word.as_str() == "-" || !word.starts_with('-'));
                let target = match operand.map(String::as_str) {
                    Some("-") if program == "cd" => state.previous_directory.clone(),
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
                if let Some(target) = target.filter(|target| target.is_dir()) {
                    if state
                        .flow_stack
                        .iter()
                        .any(|flow| matches!(flow, VariableFlow::Loop { .. }))
                        && !matches!(first, "while" | "until")
                    {
                        // A loop body may run zero or many times. The
                        // directory after its conditional cd is not a
                        // single known path at any later body point.
                        for flow in &mut state.flow_stack {
                            if let VariableFlow::Loop { changed, .. } = flow {
                                *changed = true;
                            }
                        }
                        state.directory = None;
                        state.previous_directory = None;
                        unknown_context = true;
                        continue;
                    }
                    if conditional_continuation
                        || commands
                            .get(command_no + 1)
                            .and_then(|words| words.first())
                            .is_some_and(|word| matches!(word.as_str(), "\0and" | "\0or"))
                    {
                        state.conditional_list_changed = true;
                    }
                    for flow in &mut state.flow_stack {
                        if let VariableFlow::Loop { changed, .. } = flow {
                            *changed = true;
                        }
                    }
                    if program == "pushd" {
                        state
                            .directory_stack
                            .push((Some(base.clone()), state.previous_directory.clone()));
                    }
                    state.previous_directory = Some(base);
                    state.directory = Some(target);
                } else {
                    for flow in &mut state.flow_stack {
                        if let VariableFlow::Loop { changed, .. } = flow {
                            *changed = true;
                        }
                    }
                    state.directory = None;
                    state.previous_directory = None;
                    unknown_context = true;
                }
            }
            "popd" => {
                if state
                    .flow_stack
                    .iter()
                    .any(|flow| matches!(flow, VariableFlow::Loop { .. }))
                {
                    for flow in &mut state.flow_stack {
                        if let VariableFlow::Loop { changed, .. } = flow {
                            *changed = true;
                        }
                    }
                    state.directory = None;
                    state.previous_directory = None;
                    unknown_context = true;
                } else if let Some((saved, previous)) = state.directory_stack.pop() {
                    state.directory = saved;
                    state.previous_directory = previous;
                }
            }
            _ => {}
        }
    }
    (expanded_commands, bases, unknown_context)
}

#[path = "workflow_acceptance_executability_mutation_paths_commands.rs"]
mod command_scopes;
pub(super) use command_scopes::command_directories;

#[cfg(test)]
#[path = "workflow_acceptance_executability_mutation_paths_scope_tests.rs"]
mod scope_tests;
