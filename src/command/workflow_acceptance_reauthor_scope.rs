//! Scope construction for acceptance re-author calls.

use std::fs;
use std::path::{Path, PathBuf};

/// Where the author reads.
pub(crate) struct AuthorScope {
    pub(crate) prd_path: PathBuf,
    pub(crate) project_root: PathBuf,
    pub(crate) repository_root: PathBuf,
    pub(crate) task_universe: archon_workflow::task_universe::WorkflowV2TaskUniverse,
    pub(crate) task_root: PathBuf,
    pub(crate) baseline: Option<super::super::executability::Baseline>,
    pub(crate) project_input_excludes: Vec<PathBuf>,
    pub(crate) build_cache: bool,
}

impl AuthorScope {
    pub(crate) fn covered_texts(
        &self,
    ) -> anyhow::Result<std::collections::BTreeMap<String, String>> {
        let text = fs::read_to_string(&self.prd_path)?;
        Ok(archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(&text))
    }

    /// Resolve the paths the author prompt says it may read.
    pub(crate) fn read_roots(&self) -> Result<Vec<String>, String> {
        for (field, path) in [
            ("prd_path", &self.prd_path),
            ("project_root", &self.project_root),
            ("repository_root", &self.repository_root),
        ] {
            if path.as_os_str().to_string_lossy().trim().is_empty() {
                return Err(format!("acceptance re-author scope: {field} is empty"));
            }
        }
        if self.prd_path.is_dir() {
            return Err(format!(
                "acceptance re-author scope: prd_path {} is a directory, not a PRD file",
                self.prd_path.display()
            ));
        }
        let mut roots: Vec<String> = Vec::new();
        for (field, path) in [
            ("prd_path", &self.prd_path),
            ("project_root", &self.project_root),
        ] {
            if path.starts_with(&self.repository_root) {
                continue;
            }
            let path = std::path::absolute(path)
                .map_err(|error| {
                    format!(
                        "acceptance re-author scope: {field} {} cannot be made absolute: {error}",
                        path.display()
                    )
                })?
                .display()
                .to_string();
            if !roots.contains(&path) {
                roots.push(path);
            }
        }
        Ok(roots)
    }

    /// Task declarations are advisory context for the re-author prompt. Keep
    /// usable declarations when one frozen task body is malformed.
    pub(crate) fn for_task_set(
        project_root: &Path,
        tasks_root: &Path,
        prd_path: &Path,
        build_cache: bool,
    ) -> std::result::Result<Self, String> {
        let task_files =
            archon_workflow::task_universe::task_files_under(tasks_root).map_err(|error| {
                format!(
                    "acceptance re-author task root {}: {error}",
                    tasks_root.display()
                )
            })?;
        let mut tasks = Vec::new();
        for path in task_files {
            let raw = fs::read_to_string(&path).map_err(|error| {
                format!(
                    "acceptance re-author task file {}: could not read: {error}",
                    path.display()
                )
            })?;
            match archon_workflow::task_universe::parsing::parse_task_file(&path, &raw) {
                Ok(task) => tasks.push(task),
                Err(error) => eprintln!(
                    "acceptance re-author skipped malformed task file {}: {error}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
            }
        }
        let task_universe = archon_workflow::task_universe::WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".into(),
            source_roots: vec![tasks_root.display().to_string()],
            tasks,
        };
        // An unavailable site is reported by the probe as host-unproven;
        // constructing reading scope must not turn it into a hard failure.
        let binding = crate::command::acceptance_scratch_policy::capture(project_root, tasks_root)
            .ok()
            .flatten();
        let repository_root =
            super::super::live_root::freeze_repository(project_root, tasks_root, binding.as_ref())?;
        let baseline =
            super::super::executability::Baseline::for_task_set(&repository_root, tasks_root);
        Ok(Self {
            prd_path: prd_path.to_path_buf(),
            project_root: project_root.to_path_buf(),
            repository_root,
            task_universe,
            task_root: tasks_root.to_path_buf(),
            baseline,
            // Candidate discovery is advisory and only needed when rendering
            // a baseline-pass prompt. Load its policy there, not while
            // constructing the scope used by refusal paths.
            project_input_excludes: Vec::new(),
            build_cache,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_task_set_without_cache(
        project_root: &Path,
        tasks_root: &Path,
        prd_path: &Path,
    ) -> std::result::Result<Self, String> {
        Self::for_task_set(project_root, tasks_root, prd_path, false)
    }
}

pub(crate) fn site_uses_build_cache(project_root: &Path, tasks_root: &Path) -> bool {
    crate::command::acceptance_scratch_policy::capture(project_root, tasks_root)
        .ok()
        .flatten()
        .is_some()
}

#[allow(dead_code)]
pub(crate) fn for_base_copy(
    copy: &super::super::executability::BaseCopy,
    prd_path: &Path,
    task_root: &Path,
    task_universe: archon_workflow::task_universe::WorkflowV2TaskUniverse,
) -> AuthorScope {
    AuthorScope {
        prd_path: prd_path.into(),
        project_root: copy.project().into(),
        repository_root: copy.repository().into(),
        task_universe,
        task_root: task_root.into(),
        baseline: Some(super::super::executability::Baseline {
            origin: super::super::executability::BaselineOrigin::RecordedBase,
            commit: copy.commit().into(),
            repository: copy.repository().into(),
        }),
        project_input_excludes: Vec::new(),
        build_cache: false,
    }
}
#[allow(dead_code)]
pub(crate) fn confined_read_roots(
    scope: &AuthorScope,
    copy_root: &Path,
) -> Result<Vec<String>, String> {
    let canonical = |p: &Path| archon_shell::paths::canonicalize(p).map_err(|e| e.to_string());
    let root = canonical(copy_root)?;
    for (field, path) in [
        ("repository_root", &scope.repository_root),
        ("project_root", &scope.project_root),
    ] {
        if !canonical(path)?.starts_with(&root) {
            return Err(format!(
                "acceptance author scope: {field} {} lies outside the base copy {}",
                path.display(),
                copy_root.display()
            ));
        }
    }
    let roots = scope.read_roots()?;
    for path in &roots {
        if Path::new(path) != scope.prd_path && !canonical(Path::new(path))?.starts_with(&root) {
            return Err(format!(
                "acceptance author scope: read_root {path} lies outside the base copy {}",
                copy_root.display()
            ));
        }
    }
    Ok(roots)
}
#[cfg(test)]
#[path = "workflow_acceptance_reauthor_base_scope_tests.rs"]
mod base_scope_tests;
