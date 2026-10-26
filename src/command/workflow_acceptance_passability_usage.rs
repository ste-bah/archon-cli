//! Deterministic recognition of commands rejected by an external program.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::evidence::{Redactor, STDERR_BYTES, STDOUT_BYTES};
use super::executability::BaselineRuns;
use super::*;

/// Findings for replacement checks that cannot pass on their baseline tree.
pub(crate) async fn cannot_pass_findings(
    client: &dyn WorkflowLlmClient,
    model: &str,
    repository_root: &Path,
    project_root: &Path,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    runs: &BaselineRuns,
    prd_text: &str,
    audit_dir: Option<&Path>,
) -> anyhow::Result<BTreeMap<String, String>> {
    let requirements =
        archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text);
    let redactor = Redactor::for_runs(runs);
    let candidates =
        super::candidates(client, model, contract, ids, runs, &requirements, &redactor);
    if candidates.is_empty() {
        return Ok(BTreeMap::new());
    }
    let resume = FreezeResume::none();
    let mut refuted = BTreeMap::new();
    let mut pending = Vec::new();
    for candidate in candidates {
        let Some(command) = check_command(contract, &candidate.id) else {
            pending.push(candidate);
            continue;
        };
        if let Some((program, path, rejected_line)) =
            external_usage_error(runs, &candidate.id, repository_root, project_root, command)
        {
            let rejected_line = super::evidence::inert(&rejected_line);
            let resolved = path.display().to_string();
            let rule = format!(
                "{program} ({resolved}, outside the repository) rejected the check's command line: {rejected_line}"
            );
            let reason = format!(
                "nothing the repository implements can change how {program} parses its arguments; fix the invocation"
            );
            let decision = Decision {
                verdict: JudgeDecision::Refuted,
                counterexample: rejected_line,
                reason: reason.clone(),
                sampling: Some(serde_json::json!({"provenance": "host"})),
            };
            if let Some(directory) = audit_dir {
                super::audit::write(
                    directory,
                    &candidate.key,
                    &candidate.id,
                    &decision,
                    candidate.evidence.exit_code,
                )
                .with_context(|| format!("writing passability audit for check {}", candidate.id))?;
            }
            let text = super::evidence::finding_text(
                &candidate.id,
                CANNOT_PASS,
                &candidate.evidence,
                &rule,
                &reason,
            );
            refuted.insert(candidate.id.clone(), (decision, text));
        } else {
            pending.push(candidate);
        }
    }
    if !pending.is_empty() {
        refuted.extend(
            super::assess(
                client, model, contract, &pending, &redactor, None, &resume, audit_dir,
            )
            .await?,
        );
    }
    Ok(refuted
        .into_iter()
        .map(|(id, (_, text))| (id, text))
        .collect())
}

pub(super) fn check_command<'a>(contract: &'a AcceptanceContract, id: &str) -> Option<&'a str> {
    super::entries(contract)
        .find(|entry| entry.id == id)
        .and_then(|entry| match &entry.check {
            archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } => {
                Some(command.as_str())
            }
            _ => None,
        })
}

/// Return the program, its resolved executable and parser error only when a
/// bounded baseline excerpt proves that an external executable rejected it.
pub(super) fn external_usage_error(
    runs: &BaselineRuns,
    id: &str,
    repository: &Path,
    project_root: &Path,
    command: &str,
) -> Option<(String, PathBuf, String)> {
    let search_path = runs
        .environment
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"))?;
    external_usage_error_on_path(runs, id, repository, project_root, command, &search_path)
}

fn external_usage_error_on_path(
    runs: &BaselineRuns,
    id: &str,
    repository: &Path,
    project_root: &Path,
    command: &str,
    search_path: &std::ffi::OsStr,
) -> Option<(String, PathBuf, String)> {
    let result = runs.failures.get(id)?;
    let redactor = Redactor::for_runs(runs);
    let stderr = super::evidence::program_output(&result.stderr, &redactor, STDERR_BYTES);
    let stdout = super::evidence::program_output(&result.stdout, &redactor, STDOUT_BYTES);
    let output = format!("{stderr}\n{stdout}");
    // Distance to the `Usage:` line counts text lines only: clap separates
    // the error, its tip and the usage with blank lines.
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let (index, error) = lines
        .iter()
        .enumerate()
        .find_map(|(index, line)| parser_error(line).map(|error| (index, error)))?;
    if invokes_script_or_module(command) {
        return None;
    }
    let usage = (0..lines.len())
        .filter(|at| at.abs_diff(index) <= 3)
        .filter_map(|at| usage_program(lines[at]).map(|program| (at.abs_diff(index), program)))
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, usage)| usage);
    if usage.as_ref().is_some_and(|usage| usage.script_argument) {
        return None;
    }
    let program = if let Some(program) = error_prefix(error) {
        program.to_string()
    } else {
        let usage = usage?;
        if Path::new(&usage.token).components().count() > 1 {
            return None;
        }
        usage.token
    };
    if !valid_identifier(&program) {
        return None;
    }
    let declared = declared_binaries(repository)?;
    if declared.contains(&program) {
        return None;
    }
    let resolved = resolve_executable(&program, search_path)?;
    let canonical = resolved.canonicalize().ok()?;
    let root = repository.canonicalize().ok()?;
    let project = project_root.canonicalize().ok()?;
    if canonical.starts_with(&root) || canonical.starts_with(&project) {
        return None;
    }
    Some((program, resolved, error.to_string()))
}

fn invokes_script_or_module(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else {
        return false;
    };
    if Path::new(program).components().count() > 1 {
        return true;
    }
    words
        .next()
        .is_some_and(|argument| argument == "-m" || argument.contains('/'))
}

fn parser_error(line: &str) -> Option<&str> {
    std::iter::once(line)
        .chain(
            line.match_indices(": ")
                .map(|(index, separator)| &line[index + separator.len()..]),
        )
        .find(|candidate| parser_error_at_start(candidate))
}

fn parser_error_at_start(line: &str) -> bool {
    if let Some((program, message)) = line.split_once(": error:") {
        return valid_identifier(program) && syntax_refusal(message);
    }
    if let Some(message) = line.strip_prefix("error:") {
        return syntax_refusal(message);
    }
    ["illegal option", "unrecognized option", "invalid option"]
        .iter()
        .any(|phrase| getopt_error(line, phrase))
}

fn syntax_refusal(message: &str) -> bool {
    let message = message.trim_start().to_ascii_lowercase();
    [
        "unexpected argument",
        "unrecognized argument",
        "unrecognized arguments",
        "unrecognized subcommand",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
        || ["illegal option", "unrecognized option"]
            .iter()
            .any(|prefix| message.starts_with(prefix))
        || message.strip_prefix("invalid option ").is_some_and(|rest| {
            rest.trim_start()
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '-' | '\'' | '"' | '`'))
        })
}

fn error_prefix(line: &str) -> Option<&str> {
    let (program, _) = line.split_once(": error:")?;
    valid_identifier(program).then_some(program)
}

fn getopt_error(line: &str, phrase: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let (program, rest) = match lower.find(phrase) {
        Some(index) => (&line[..index], &lower[index..]),
        None => return false,
    };
    valid_identifier(program.trim().trim_end_matches(':'))
        && ["illegal option", "unrecognized option", "invalid option"]
            .iter()
            .any(|shape| rest.starts_with(shape))
        && (phrase != "invalid option"
            || rest.strip_prefix("invalid option").is_some_and(|rest| {
                rest.trim_start()
                    .chars()
                    .next()
                    .is_some_and(|ch| matches!(ch, '-' | '\'' | '"' | '`'))
            }))
}

struct UsageProgram {
    token: String,
    script_argument: bool,
}

fn usage_program(line: &str) -> Option<UsageProgram> {
    let (prefix, rest) = line.split_once(':')?;
    if !prefix.eq_ignore_ascii_case("usage") {
        return None;
    }
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let first = tokens.first()?.trim_matches(['\'', '"', '`']);
    if !valid_identifier(first) {
        return None;
    }
    let script_argument = tokens
        .get(1)
        .is_some_and(|token| *token == "-m" || token.contains('/'));
    let token = if script_argument {
        tokens[1].trim_matches(['\'', '"', '`']).to_string()
    } else {
        first.to_string()
    };
    Some(UsageProgram {
        token,
        script_argument,
    })
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+'))
}

fn resolve_executable(program: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|directory| directory.join(program))
        .find(|candidate| {
            candidate.is_file()
                && std::fs::metadata(candidate).is_ok_and(|metadata| executable_file(&metadata))
        })
}

#[cfg(unix)]
fn executable_file(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable_file(_metadata: &std::fs::Metadata) -> bool {
    true
}

// This read-only pass enumerates entries once, prunes hidden/generated/vendor
// trees, and opens only manifests and immediate source-bin entries.
fn declared_binaries(repository: &Path) -> Option<BTreeSet<String>> {
    let root = repository.canonicalize().ok()?;
    let mut names = BTreeSet::new();
    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || !skip_directory(entry.file_name().to_str().unwrap_or_default())
        })
    {
        let entry = entry.ok()?;
        if entry.file_type().is_dir()
            && entry.file_name() == "bin"
            && entry.path().parent()?.file_name()?.to_str()? == "src"
        {
            implicit_bin_directory(entry.path(), &mut names)?;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        match entry.file_name().to_str()? {
            "Cargo.toml" => toml_names(entry.path(), &mut names)?,
            "package.json" => json_names(entry.path(), &mut names)?,
            "pyproject.toml" => script_names(entry.path(), &mut names)?,
            _ => {}
        }
    }
    Some(names)
}

fn skip_directory(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "target" | "node_modules" | "dist" | "build" | "vendor"
        )
}

fn toml_names(path: &Path, names: &mut BTreeSet<String>) -> Option<()> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = toml::from_str::<toml::Value>(&text).ok()?;
    let table = value.as_table()?;
    if let Some(name) = table
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
    {
        names.insert(name.as_str()?.to_string());
    }
    if let Some(bins) = table.get("bin") {
        for bin in bins.as_array()? {
            names.insert(bin.as_table()?.get("name")?.as_str()?.to_string());
        }
    }
    Some(())
}

fn implicit_bin_directory(bin: &Path, names: &mut BTreeSet<String>) -> Option<()> {
    for entry in std::fs::read_dir(bin).ok()? {
        let entry = entry.ok()?;
        let kind = entry.file_type().ok()?;
        let name = entry.file_name().into_string().ok()?;
        if kind.is_file() && Path::new(&name).extension().is_some_and(|ext| ext == "rs") {
            names.insert(Path::new(&name).file_stem()?.to_str()?.to_string());
        } else if kind.is_dir() && entry.path().join("main.rs").is_file() {
            names.insert(name);
        }
    }
    Some(())
}

fn json_names(path: &Path, names: &mut BTreeSet<String>) -> Option<()> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    value.as_object()?;
    let Some(bin) = value.get("bin") else {
        return Some(());
    };
    if bin.is_string() {
        if let Some(package) = value.get("name").and_then(serde_json::Value::as_str) {
            names.insert(package.rsplit('/').next()?.to_string());
        }
    } else if let Some(entries) = bin.as_object() {
        names.extend(
            entries
                .keys()
                .map(|key| key.rsplit('/').next().unwrap_or(key).to_string()),
        );
    } else {
        return None;
    }
    Some(())
}

fn script_names(path: &Path, names: &mut BTreeSet<String>) -> Option<()> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = toml::from_str::<toml::Value>(&text).ok()?;
    for key in ["scripts"] {
        if let Some(scripts) = value
            .get("project")
            .and_then(|project| project.get(key))
            .and_then(toml::Value::as_table)
        {
            names.extend(scripts.keys().cloned());
        }
        if let Some(scripts) = value
            .get("tool")
            .and_then(|tool| tool.get("poetry"))
            .and_then(|poetry| poetry.get(key))
            .and_then(toml::Value::as_table)
        {
            names.extend(scripts.keys().cloned());
        }
    }
    Some(())
}

#[cfg(test)]
#[path = "workflow_acceptance_passability_usage_regression_tests.rs"]
mod regression_tests;
#[cfg(test)]
#[path = "workflow_acceptance_passability_usage_tests.rs"]
mod tests;
