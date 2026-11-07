//! Toolchain test inventories, selected on the pre-implementation tree.
use super::verdict_shell::{Simple, simple_commands};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Runner {
    Cargo,
    Nextest,
    Pytest,
}

#[derive(Clone, Debug)]
pub(crate) struct Invocation {
    pub(crate) runner: Runner,
    pub(crate) line: String,
    pub(crate) list: String,
    pub(crate) filter: String,
    /// All positional filters from this command, sharing its target and match mode.
    pub(crate) filters: Vec<String>,
    pub(crate) exact: bool,
}

impl Invocation {
    fn matches(&self, name: &str, filter: &str) -> bool {
        if self.exact {
            name == filter
                || (self.runner == Runner::Pytest
                    && (name.starts_with(&format!("{filter}["))
                        || name.starts_with(&format!("{filter}::"))))
        } else if self.runner == Runner::Pytest {
            name.to_ascii_lowercase()
                .contains(&filter.to_ascii_lowercase())
        } else {
            name.contains(filter)
        }
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
fn rendered(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Only tabled runner forms are probed; unresolved shell directories stay unknown.
pub(crate) fn invocations(text: &str) -> Vec<Invocation> {
    let mut found = Vec::new();
    for (command, cwd) in located_commands(text) {
        let Some(cwd) = &cwd else { continue };
        let Some(program) = &command.program else {
            continue;
        };
        let name = program.rsplit('/').next().unwrap_or(program);
        let args = &command.args;
        if args.iter().any(|arg| super::verdict_shell::expands(arg)) {
            continue;
        }
        let (runner, start) = match (name, args.first().map(String::as_str)) {
            ("cargo", Some("test")) => (Runner::Cargo, 1),
            ("cargo", Some("nextest")) if args.get(1).is_some_and(|arg| arg == "run") => {
                (Runner::Nextest, 2)
            }
            ("pytest" | "pytest-3", _) => (Runner::Pytest, 0),
            _ => continue,
        };
        let before = args
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(args.len());
        let mut selected = Vec::new();
        let mut filters = Vec::new();
        let mut index = start;
        let mut supported = true;
        while index < before {
            let arg = &args[index];
            if [
                "-p",
                "--package",
                "--manifest-path",
                "--test",
                "--bin",
                "--example",
                "--features",
                "-F",
                "--target-dir",
                "--exclude",
                "--target",
                "--profile",
            ]
            .contains(&arg.as_str())
            {
                let Some(value) = args.get(index + 1) else {
                    supported = false;
                    break;
                };
                selected.extend([arg.clone(), value.clone()]);
                index += 2;
                continue;
            }
            if arg == "-k" && runner == Runner::Pytest {
                if let Some(value) = args.get(index + 1) {
                    if value
                        .chars()
                        .any(|c| c.is_whitespace() || matches!(c, '(' | ')'))
                    {
                        supported = false;
                        break;
                    }
                    filters.push(value.clone());
                }
                index += 2;
                continue;
            }
            if arg == "-E" || arg == "--filter-expr" {
                supported = false;
                break;
            }
            if arg.starts_with('-') {
                // Listing preserves package/target/feature selection, not run-only reporting.
                if !matches!(
                    arg.as_str(),
                    "-q" | "--quiet" | "--nocapture" | "--exact" | "-v" | "--verbose"
                ) {
                    selected.push(arg.clone());
                }
            } else {
                filters.push(arg.clone());
            }
            index += 1;
        }
        if !supported || filters.len() > 1 {
            continue;
        }
        if runner != Runner::Pytest && before < args.len() {
            let mut skip = false;
            for arg in &args[before + 1..] {
                if std::mem::take(&mut skip) {
                    continue;
                }
                if matches!(
                    arg.as_str(),
                    "--test-threads"
                        | "--skip"
                        | "--format"
                        | "--logfile"
                        | "-Z"
                        | "--shuffle-seed"
                        | "--color"
                ) {
                    skip = true;
                } else if !arg.starts_with('-') {
                    filters.push(arg.clone());
                }
            }
        }
        if filters.is_empty() {
            filters.push(String::new());
        }
        for mut filter in filters.clone() {
            let exact = (runner == Runner::Pytest && !args.iter().any(|arg| arg == "-k"))
                || args[before..].iter().any(|arg| arg == "--exact");
            let mut list_args = match runner {
                Runner::Cargo => vec!["test".into()],
                Runner::Nextest => vec!["nextest".into(), "list".into()],
                Runner::Pytest => Vec::new(),
            };
            list_args.extend(selected.clone());
            match runner {
                Runner::Cargo => list_args.extend(["--".into(), "--list".into()]),
                Runner::Nextest => list_args.extend(["--message-format".into(), "json".into()]),
                Runner::Pytest => {
                    if filter.contains("::") {
                        list_args.push(filter.split("::").next().unwrap_or_default().into());
                    } else if !filter.is_empty() && !args.iter().any(|arg| arg == "-k") {
                        list_args.push(filter.clone());
                        filter.clear();
                    }
                    list_args.extend(["--collect-only".into(), "-q".into()]);
                }
            }
            let prefix = cwd
                .iter()
                .map(|dir| format!("cd {} && ", quote(dir)))
                .collect::<String>();
            found.push(Invocation {
                runner: runner.clone(),
                line: rendered(program, args),
                list: format!("{prefix}{}", rendered(program, &list_args)),
                filter,
                filters: filters.clone(),
                exact,
            });
        }
    }
    found
}

/// Directory changes and their scope, without evaluating shell expansions.
fn located_commands(text: &str) -> Vec<(Simple, Option<Vec<String>>)> {
    located_commands_with_guards(text, true)
}

fn located_commands_with_guards(
    text: &str,
    reject_guards: bool,
) -> Vec<(Simple, Option<Vec<String>>)> {
    let mut cwd = Some(Vec::new());
    let mut previous = None;
    let mut stack = Vec::new();
    let mut scopes = Vec::new();
    let mut found = Vec::new();
    let text = super::verdict_shell::without_heredoc_bodies(text);
    let parts = super::verdict::setup_path::flow::segments(&text);
    if reject_guards
        && parts.iter().any(|(body, op)| {
            op == "||"
                || body.trim_start().starts_with('!')
                || body
                    .split_whitespace()
                    .next()
                    .is_some_and(|word| matches!(word, "if" | "while" | "until" | "for" | "case"))
        })
    {
        return found;
    }
    for (body, op) in parts {
        for command in simple_commands(&body) {
            match command.word.as_str() {
                "cd" | "pushd" => {
                    let next = match command.args.first().map(String::as_str) {
                        Some("-") => previous.clone(),
                        Some(path) if !super::verdict_shell::expands(path) => {
                            cwd.clone().map(|mut parts: Vec<String>| {
                                parts.push(path.into());
                                parts
                            })
                        }
                        _ => None,
                    };
                    if command.word == "pushd" {
                        stack.push(cwd.clone());
                    }
                    previous = cwd.clone();
                    cwd = next;
                }
                "popd" => {
                    previous = cwd.clone();
                    cwd = stack.pop().flatten();
                }
                _ => found.push((command, cwd.clone())),
            }
        }
        match op.as_str() {
            "(" => scopes.push((cwd.clone(), previous.clone(), stack.clone())),
            ")" => {
                if let Some((dir, old, saved)) = scopes.pop() {
                    cwd = dir;
                    previous = old;
                    stack = saved;
                }
            }
            _ => {}
        }
    }
    found
}

pub(crate) fn shell_commands(text: &str) -> Vec<(String, Vec<String>)> {
    simple_commands(text)
        .into_iter()
        .map(|command| (command.word, command.args))
        .collect()
}

pub(crate) fn cargo_multi_names(text: &str) -> Vec<String> {
    simple_commands(text)
        .into_iter()
        .filter_map(|command| {
            let program = command.program.as_deref()?;
            if program.rsplit('/').next()? != "cargo" || command.args.first()? != "test" {
                return None;
            }
            let mut names = 0;
            let mut skip = false;
            for arg in command.args.iter().skip(1).take_while(|arg| *arg != "--") {
                if std::mem::take(&mut skip) {
                    continue;
                }
                if [
                    "-p",
                    "--package",
                    "--manifest-path",
                    "--test",
                    "--bin",
                    "--example",
                    "--features",
                    "-F",
                    "--target-dir",
                    "--exclude",
                    "--target",
                    "--profile",
                    "-j",
                    "--jobs",
                    "--color",
                ]
                .contains(&arg.as_str())
                {
                    skip = true;
                } else if !arg.starts_with('-') {
                    names += 1;
                }
            }
            (names > 1).then(|| rendered(program, &command.args))
        })
        .collect()
}

/// A failed or unrecognized listing is unavailable, never evidence of no tests.
pub(crate) fn inventory(runner: &Runner, output: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    match runner {
        Runner::Cargo => {
            for line in output.lines() {
                if let Some(name) = line
                    .strip_suffix(": test")
                    .or_else(|| line.strip_suffix(": benchmark"))
                {
                    names.push(name.to_string());
                }
            }
            if names.is_empty() && !output.contains("0 tests") {
                return None;
            }
        }
        Runner::Nextest => {
            let value: serde_json::Value = serde_json::from_str(output).ok()?;
            for suite in value.get("rust-suites")?.as_object()?.values() {
                names.extend(suite.get("testcases")?.as_object()?.keys().cloned());
            }
        }
        Runner::Pytest => {
            names.extend(
                output
                    .lines()
                    .filter(|line| line.contains("::") && !line.starts_with(' '))
                    .map(str::to_string),
            );
            if names.is_empty() && !output.contains("no tests collected") {
                return None;
            }
        }
    }
    names.sort();
    names.dedup();
    Some(names)
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<_> = b.chars().collect();
    let mut row: Vec<_> = (0..=b.len()).collect();
    for (i, a) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(old + 1)
                .min(diagonal + usize::from(a != *b));
            diagonal = old;
        }
    }
    row[b.len()]
}

pub(crate) fn unavailable_note(id: &str, invocation: &str) -> String {
    format!("check '{id}': inventory unavailable for {invocation}")
}
pub(crate) fn nearest_paths(invocation: &Invocation, names: &[String]) -> Vec<String> {
    let mut nearest = names.to_vec();
    nearest.sort_by_key(|name| (distance(&invocation.filter, name), name.clone()));
    nearest.truncate(3);
    nearest
}

pub(crate) fn mismatch_for_run(
    exit: Option<i32>,
    invocation: &Invocation,
    names: Option<&[String]>,
) -> Option<String> {
    if exit != Some(0) {
        return None;
    }
    mismatch(invocation, names?)
}

pub(crate) fn mismatch(invocation: &Invocation, names: &[String]) -> Option<String> {
    if invocation.filter.is_empty()
        || names
            .iter()
            .any(|name| invocation.matches(name, &invocation.filter))
    {
        return None;
    }
    let nearest = nearest_paths(invocation, names);
    if invocation.filters.len() > 1 {
        let others = invocation
            .filters
            .iter()
            .filter(|filter| *filter != &invocation.filter)
            .collect::<Vec<_>>();
        let matched = names
            .iter()
            .filter(|name| others.iter().any(|filter| invocation.matches(name, filter)))
            .count();
        let named = |filters: &[&String]| {
            filters
                .iter()
                .map(|filter| format!("`{filter}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Some(if matched > 0 {
            format!(
                "filter `{}` in {} matches no test in its selected base-tree target while the other named filters ({}) match {} tests; every named test must exist or the check must fail at base when it does not run; check the full test path against the nearest real test paths: {}",
                invocation.filter,
                invocation.line,
                named(&others),
                matched,
                nearest.join(", ")
            )
        } else {
            format!(
                "{} ran 0 matching tests: filters {} matched no test in their selected base-tree target; every named test must exist or the check must fail at base when it does not run; check the full test path against the nearest real test paths: {}",
                invocation.line,
                named(&invocation.filters.iter().collect::<Vec<_>>()),
                nearest.join(", ")
            )
        });
    }
    Some(format!(
        "{} ran 0 matching tests: filter `{}` matched no test in its selected base-tree target; if the work adds this test, it does not exist yet at base and the check must fail there when no test runs; otherwise check the full test path against the nearest real test paths: {}",
        invocation.line,
        invocation.filter,
        nearest.join(", ")
    ))
}

#[path = "workflow_acceptance_test_inventory_probe.rs"]
mod probe;
pub(super) use probe::findings;

#[cfg(test)]
#[path = "workflow_acceptance_test_inventory_tests.rs"]
mod tests;

pub(crate) fn located_shell_commands(
    text: &str,
) -> Vec<(String, Vec<String>, Option<Vec<String>>)> {
    located_commands(text)
        .into_iter()
        .map(|(command, cwd)| (command.word, command.args, cwd))
        .collect()
}
