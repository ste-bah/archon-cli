//! Deterministic recognition of commands rejected by an undeclared program.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::evidence::{Redactor, STDERR_BYTES, STDOUT_BYTES};
use super::executability::BaselineRuns;
use super::*;

const USAGE_ERRORS: &[(&[&str], &[&str])] = &[
    (&["unexpected argument"], &["error:"]),
    (&["unrecognized subcommand"], &["error:"]),
    (&["unrecognized arguments"], &["error:"]),
    (&["illegal option"], &[]),
    (&["unrecognized option"], &[]),
    (&["invalid option"], &[]),
];

static DECLARED: OnceLock<Mutex<HashMap<PathBuf, BTreeSet<String>>>> = OnceLock::new();

/// Findings for replacement checks that cannot pass on their baseline tree.
pub(crate) async fn cannot_pass_findings(
    client: &dyn WorkflowLlmClient,
    model: &str,
    repository_root: &Path,
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
        if let Some((program, rejected_line)) =
            external_usage_error(runs, &candidate.id, repository_root)
        {
            let rejected_line = super::evidence::inert(&rejected_line);
            let reason = format!(
                "{program} rejected the check's own command: {rejected_line}; no implementation can change how {program} parses its arguments; fix the invocation"
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
            refuted.insert(
                candidate.id.clone(),
                (
                    decision,
                    format!("check '{}': {CANNOT_PASS}: {reason}", candidate.id),
                ),
            );
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

/// Return the rejecting program and parser error when a bounded baseline
/// excerpt proves that an undeclared program rejected the invocation.
pub(super) fn external_usage_error(
    runs: &BaselineRuns,
    id: &str,
    repository: &Path,
) -> Option<(String, String)> {
    let result = runs.failures.get(id)?;
    let redactor = Redactor::for_runs(runs);
    let stderr = super::evidence::program_output(&result.stderr, &redactor, STDERR_BYTES);
    let stdout = super::evidence::program_output(&result.stdout, &redactor, STDOUT_BYTES);
    let output = format!("{stderr}\n{stdout}");
    let lines: Vec<&str> = output.lines().collect();
    // The usage line nearest the parser error names the rejecting program:
    // clap prints it after the error, argparse before it.
    let (program, error) = lines.iter().enumerate().find_map(|(index, line)| {
        let lower = line.to_ascii_lowercase();
        USAGE_ERRORS
            .iter()
            .any(|(phrases, prefixes)| {
                (prefixes.is_empty() || prefixes.iter().any(|prefix| lower.contains(prefix)))
                    && phrases.iter().any(|phrase| lower.contains(phrase))
            })
            .then(|| {
                let program = lines
                    .iter()
                    .enumerate()
                    .filter_map(|(at, line)| Some((at.abs_diff(index), usage_program(line)?)))
                    .min_by_key(|(distance, _)| *distance)
                    .map(|(_, program)| program)?;
                Some((program, line.to_string()))
            })
            .flatten()
    })?;
    let declared = declared_binaries(repository);
    (!declared.contains(&program)).then_some((program, error))
}

fn usage_program(line: &str) -> Option<String> {
    let (prefix, rest) = line.split_once(':')?;
    if !prefix.eq_ignore_ascii_case("usage") {
        return None;
    }
    let program = rest
        .split_whitespace()
        .next()?
        .trim_matches(['\'', '"', '`']);
    (!program.is_empty()).then(|| {
        Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(program)
            .to_string()
    })
}

fn declared_binaries(repository: &Path) -> BTreeSet<String> {
    let root = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.to_path_buf());
    let cache = DECLARED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(names) = cache.lock().expect("binary cache lock").get(&root) {
        return names.clone();
    }
    let mut names = BTreeSet::new();
    for entry in walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "target" | "node_modules")
                )
        })
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        match entry.file_name().to_str() {
            Some("Cargo.toml") => cargo_names(entry.path(), &mut names),
            Some("package.json") => package_names(entry.path(), &mut names),
            Some("pyproject.toml") => python_names(entry.path(), &mut names),
            _ => {}
        }
    }
    cache
        .lock()
        .expect("binary cache lock")
        .insert(root, names.clone());
    names
}

fn cargo_names(path: &Path, names: &mut BTreeSet<String>) {
    let Ok(value) = std::fs::read_to_string(path)
        .and_then(|text| toml::from_str::<toml::Value>(&text).map_err(std::io::Error::other))
    else {
        return;
    };
    let Some(table) = value.as_table() else {
        return;
    };
    if let Some(name) = table
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
    {
        names.insert(name.to_string());
    }
    if let Some(bins) = table.get("bin").and_then(toml::Value::as_array) {
        names.extend(
            bins.iter()
                .filter_map(|bin| bin.as_table()?.get("name")?.as_str().map(str::to_string)),
        );
    }
}

fn package_names(path: &Path, names: &mut BTreeSet<String>) {
    let Ok(value) = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .ok_or(())
    else {
        return;
    };
    let Some(bin) = value.get("bin") else {
        return;
    };
    if bin.is_string() {
        if let Some(package) = value.get("name").and_then(serde_json::Value::as_str) {
            names.insert(package.to_string());
        }
    } else if let Some(entries) = bin.as_object() {
        names.extend(entries.keys().cloned());
    }
}

fn python_names(path: &Path, names: &mut BTreeSet<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return;
    };
    if let Some(scripts) = value
        .get("project")
        .and_then(|project| project.get("scripts"))
        .and_then(toml::Value::as_table)
    {
        names.extend(scripts.keys().cloned());
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_passability_usage_tests.rs"]
mod tests;
