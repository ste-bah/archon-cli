use super::*;

#[cfg(test)]
pub(in crate::command::topology_lint) fn inspect(
    cwd: &Path,
    path: &Path,
    mode: archon_core::config::GateMode,
) -> TaskFileLint {
    inspect_at_path(cwd, path, mode, None)
}

pub(in crate::command::topology_lint) fn inspect_at_path(
    cwd: &Path,
    path: &Path,
    mode: archon_core::config::GateMode,
    site_path: Option<&std::ffi::OsStr>,
) -> TaskFileLint {
    let path = absolute(cwd, path);
    match std::fs::read_to_string(&path) {
        Ok(raw) => inspect_raw_at_path(cwd, &path, &raw, mode, site_path),
        Err(error) => {
            let report = format!("# topology lint — task file {}\n", shown(&path));
            let (mut blockers, mut identities) = (Vec::new(), Identities::new());
            block(
                &mut blockers,
                &mut identities,
                format!(
                    "{}: unreadable: {error}; restore the TASK file and re-run `workflow lint --task-file {}`",
                    shown(&path),
                    shown(&path)
                ),
                DeterministicDefect::new("unreadable_task_file", "task_file", "file"),
            );
            finish(report, blockers, BTreeSet::new(), identities)
        }
    }
}
