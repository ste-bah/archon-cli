//! Copy input data while retaining typed I/O causes.
use super::*;
pub(super) fn copy_data(
    root: &Path,
    relative: &Path,
    dest: &Path,
    repository: &Path,
    task_root: Option<&Path>,
    remaining: &mut u64,
) -> Result<(), Unrun> {
    let root = archon_shell::paths::canonicalize(root).map_err(|error| {
        let text = format!(
            "could not canonicalize project root {}: {error}",
            root.display()
        );
        Unrun::error(error.into(), text)
    })?;
    let task_root = task_root_relative(&root, task_root)?;
    let repository = archon_shell::paths::canonicalize(repository).map_err(|error| {
        let text = format!("{}: {error}", repository.display());
        Unrun::error(error.into(), text)
    })?;
    copy_data_relative(
        &root,
        relative,
        dest,
        nested_repository(&root, &repository).as_deref(),
        task_root.as_deref(),
        remaining,
    )
}

fn copy_data_relative(
    root: &Path,
    relative: &Path,
    dest: &Path,
    nested: Option<&Path>,
    task_root: Option<&Path>,
    remaining: &mut u64,
) -> Result<(), Unrun> {
    let io = |path: &Path, error: std::io::Error| {
        let text = format!("{}: {error}", path.display());
        Unrun::error(error.into(), text)
    };
    let source = root.join(relative);
    if !relative.as_os_str().is_empty() && excluded(relative, task_root) {
        return Ok(());
    }
    if nested_repository_skipped(relative, nested) {
        return Ok(());
    }
    let meta = source.symlink_metadata().map_err(|e| io(&source, e))?;
    let target = dest.join(relative);
    if meta.is_dir() {
        std::fs::create_dir_all(&target).map_err(|e| io(&target, e))?;
        for item in std::fs::read_dir(&source).map_err(|e| io(&source, e))? {
            let item = item.map_err(|e| io(&source, e))?;
            copy_data_relative(
                root,
                &relative.join(item.file_name()),
                dest,
                nested,
                task_root,
                remaining,
            )?;
        }
    } else if meta.is_file() {
        if meta.len() > *remaining {
            return Err(Unrun::unknown(format!(
                "the project data exceeds the {} GiB a probe copies without a scratch policy; configure [workflow.acceptance_execution] with the project_inputs the checks read",
                PROJECT_COPY_BYTES >> 30
            )));
        }
        *remaining -= meta.len();
        std::fs::copy(&source, &target).map_err(|e| io(&source, e))?;
    }
    Ok(())
}
