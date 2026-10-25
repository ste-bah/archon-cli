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
        let repository_root =
            super::super::live_root::recorded_repository(project_root, tasks_root)?;
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
            build_cache: false,
        })
    }
}
