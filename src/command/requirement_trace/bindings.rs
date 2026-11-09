use super::*;

/// Read every `TASK-*.md` in a directory.
///
/// A file that fails to parse aborts with its own name in the message. A task
/// set that cannot be read in full cannot answer "is every requirement
/// claimed", and answering it anyway from a partial read is how an unclaimed
/// requirement disappears.
#[cfg(test)]
pub(super) fn load_bindings(dir: &Path) -> Result<Vec<TaskBinding>> {
    let (bindings, findings) = load_bindings_with_findings(dir)?;
    if findings.is_empty() {
        return Ok(bindings);
    }
    Err(anyhow!(
        "traceability input error:\n  {}",
        findings.join("\n  ")
    ))
}

pub(super) fn load_bindings_with_findings_at_path(
    dir: &Path,
    site_path: &std::ffi::OsStr,
) -> Result<(Vec<TaskBinding>, Vec<String>)> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading task directory {}", dir.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let is_task = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("TASK-") && name.ends_with(".md"));
        if is_task {
            paths.push(path);
        }
    }
    paths.sort();

    if paths.is_empty() {
        return Ok((
            Vec::new(),
            vec![format!(
                "task directory {} contains zero TASK-*.md files; add the decomposed TASK files or correct --tasks to the directory that contains them",
                dir.display()
            )],
        ));
    }

    let mut bindings = Vec::with_capacity(paths.len());
    let mut findings = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading task file {}", path.display()))?;
        let source = path.display().to_string().replace('\\', "/");
        match tasks::parse_task_binding_with_path(&raw, &source, &site_path) {
            Ok(binding) => bindings.push(binding),
            Err(error) => findings.push(format!(
                "{error}; rewrite the named YAML field exactly as shown, then re-run the same trace command"
            )),
        }
    }
    findings.sort();
    findings.dedup();
    Ok((bindings, findings))
}

#[cfg(test)]
#[path = "binding_site_tests.rs"]
mod test_site;

#[cfg(test)]
pub(super) fn load_bindings_with_findings(dir: &Path) -> Result<(Vec<TaskBinding>, Vec<String>)> {
    let (_site, path) = test_site::command_site();
    load_bindings_with_findings_at_path(dir, &path)
}
