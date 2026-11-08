//! Base-tree evidence for an invocation that cannot locate its own setup.
use super::*;
use std::path::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    Package,
    Test,
    Bin,
    Path,
}

fn missing(output: &str) -> Option<(Missing, String)> {
    for (kind, pattern) in [
        (
            Missing::Package,
            r#"package ID specification [`'\"]([^`'\"]+)[`'\"] did not match any packages"#,
        ),
        (
            Missing::Test,
            r#"no test target named [`'\"]([^`'\"]+)[`'\"]"#,
        ),
        (
            Missing::Bin,
            r#"no bin target named [`'\"]([^`'\"]+)[`'\"]"#,
        ),
        (Missing::Path, r#"file or directory not found: ([^\n]+)"#),
        (
            Missing::Path,
            r#"manifest path [`'\"]([^`'\"]+)[`'\"] does not exist"#,
        ),
    ] {
        // Patterns are toolchain grammar, never names from a particular check.
        if let Some(capture) = regex::Regex::new(pattern)
            .expect("static pattern")
            .captures(output)
        {
            return Some((
                kind,
                capture[1].trim().trim_matches(['\'', '"', '`']).to_string(),
            ));
        }
    }
    output
        .contains("could not find `Cargo.toml`")
        .then(|| (Missing::Path, "Cargo.toml".into()))
}

fn git(repository: &Path, args: &[&str]) -> Option<String> {
    let output = archon_shell::spawn::git_command(repository)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn normalized(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// `None` means the base could not be inspected: never manufacture absence.
fn exists(
    repository: &Path,
    commit: &str,
    kind: Missing,
    name: &str,
    command: &str,
    cwd: &Path,
) -> Option<bool> {
    let paths = git(repository, &["ls-tree", "-r", "--name-only", commit])?;
    let paths: Vec<_> = paths.lines().collect();
    if kind == Missing::Path {
        let mut cwd = cwd.to_path_buf();
        for invocation in super::super::executability::test_inventory::shell_commands(command) {
            if invocation.0 == "cd" {
                let path = invocation.1.first()?;
                if path.contains(['$', '~', '`']) || path == "-" {
                    return None;
                }
                cwd.push(path);
            }
        }
        let path = normalized(&cwd.join(name.split("::").next()?))?;
        return Some(
            paths
                .iter()
                .any(|entry| *entry == path || entry.starts_with(&format!("{path}/"))),
        );
    }
    let commands = super::super::executability::test_inventory::shell_commands(command);
    let selected = commands
        .iter()
        .flat_map(|(_, args)| args.windows(2))
        .find(|args| matches!(args[0].as_str(), "-p" | "--package"))
        .map(|args| args[1].as_str());
    for manifest in paths.iter().filter(|path| path.ends_with("Cargo.toml")) {
        let text = git(repository, &["show", &format!("{commit}:{manifest}")])?;
        let value: toml::Value = toml::from_str(&text).ok()?;
        let package = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str);
        if kind == Missing::Package && package == Some(name) {
            return Some(true);
        }
        if selected.is_some() && package != selected {
            continue;
        }
        let section = match kind {
            Missing::Test => "test",
            Missing::Bin => "bin",
            _ => continue,
        };
        if value
            .get(section)
            .and_then(toml::Value::as_array)
            .is_some_and(|targets| {
                targets
                    .iter()
                    .any(|target| target.get("name").and_then(toml::Value::as_str) == Some(name))
            })
        {
            return Some(true);
        }
        let directory = Path::new(manifest).parent()?;
        let implicit = match kind {
            Missing::Test => directory.join(format!("tests/{name}.rs")),
            Missing::Bin => directory.join(format!("src/bin/{name}.rs")),
            _ => unreachable!(),
        };
        let main = implicit.with_extension("").join("main.rs");
        if paths
            .iter()
            .any(|path| Path::new(path) == implicit || Path::new(path) == main)
            || (kind == Missing::Bin
                && package == Some(name)
                && paths
                    .iter()
                    .any(|path| Path::new(path) == directory.join("src/main.rs")))
        {
            return Some(true);
        }
    }
    Some(false)
}

/// Absent setup is deterministic unless a covered requirement asks to add it.
/// In that case the model sees the absence and the precise requirement together.
pub(super) fn diagnose(
    candidate: &mut Candidate,
    contract: &AcceptanceContract,
    requirements: &BTreeMap<String, String>,
    repository: &Path,
    project: &Path,
    base: &str,
) -> Option<(String, String)> {
    let entry = entries(contract).find(|entry| entry.id == candidate.id)?;
    let (command, cwd) = match &entry.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, cwd } => {
            (command.as_str(), cwd)
        }
        _ => return None,
    };
    let output = format!(
        "{}\n{}",
        candidate.evidence.stderr, candidate.evidence.stdout
    );
    let (kind, name) = missing(&output)?;
    let base_exists = match cwd {
        archon_workflow::task_set_contract::TrustedCwd::RepoRoot => {
            exists(repository, base, kind, &name, command, Path::new(""))
        }
        archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => {
            match project.strip_prefix(repository) {
                Ok(cwd) => exists(repository, base, kind, &name, command, cwd),
                Err(_) if kind == Missing::Path => Some(project.join(&name).exists()),
                Err(_) => exists(repository, base, kind, &name, command, Path::new("")),
            }
        }
    };
    if base_exists? {
        return None;
    }
    let requirement = entry
        .covers
        .iter()
        .filter_map(|id| requirements.get(id).map(|text| (id, text)))
        .find(|(_, text)| asks_to_add(text, &name));
    if let Some((id, text)) = requirement {
        candidate.shown["base_setup"] = serde_json::json!({"fact": format!("{name} is absent at base; requirement {id} says to add it"), "requirement": text});
        candidate.key = verdict_key(
            INSTRUCTION,
            serde_json::json!([candidate.key, candidate.shown["base_setup"]]),
        );
        return None;
    }
    // New test and binary targets need not be named in behavioral requirements.
    // Their absence is evidence for the judge, never a deterministic setup refutation.
    if matches!(kind, Missing::Test | Missing::Bin)
        || (kind == Missing::Path && command.contains("pytest"))
        || asks_to_add(&entry.criterion, &name)
    {
        candidate.shown["base_setup"] = serde_json::json!({"fact": format!("{name} is absent at base; judge whether the work must add it")});
        candidate.key = verdict_key(
            INSTRUCTION,
            serde_json::json!([candidate.key, candidate.shown["base_setup"]]),
        );
        return None;
    }
    let kind = match kind {
        Missing::Package => "package",
        Missing::Test => "test target",
        Missing::Bin => "binary target",
        Missing::Path => "path",
    };
    Some((
        evidence::inert(&format!(
            "the check cannot locate its own {kind} `{name}`, which is absent at base"
        )),
        evidence::inert(&format!(
            "no covered requirement asks to add `{name}`; name an existing project, manifest, package, target or test file"
        )),
    ))
}

#[path = "workflow_acceptance_requirement_addition.rs"]
mod addition;
use addition::asks_to_add;

#[path = "workflow_acceptance_passability_packages.rs"]
mod packages;
pub(in crate::command::workflow_task_set) use packages::package_findings;
