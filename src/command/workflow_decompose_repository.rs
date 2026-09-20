//! The repository a decomposition grounds its authors in (Issue-55).
//!
//! The launcher used to build its `WorkflowSpec` with no repository root and
//! tell every author to read "repository source under the project root",
//! which was the working directory. Where the project directory is not the
//! code repository, that root holds PRDs and task sets and no source, so the
//! authors globbed an empty tree and wrote tasks asserting that existing
//! files "do not exist".
//!
//! The repository is now an explicit input with three sources and no silent
//! fallback: `--repository <PATH>`, then `[workflow] repository_root`, then
//! `[workflow.acceptance_execution].repository`; with none of them the launch
//! refuses and names the ways to supply one. The working directory is never
//! assumed. The resolved path must exist and be a git checkout; a freshly
//! initialised repository with no commit is valid and records `unborn`.
//!
//! The launch records the canonical root and the base commit in
//! `<tasks>/repository.lock` once. A later launch on the same task root — a
//! frozen-chain resume — must name the same repository path; a base commit
//! that has moved is reported in the operator log and the UI, never refused
//! and never hidden.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_workflow::repository_record::{
    REPOSITORY_LOCK_FILE, REPOSITORY_RECORD_SCHEMA_VERSION, RepositoryRecordV1, git_head,
    is_git_checkout, read_repository_record, write_repository_record,
};

/// Which of the three inputs named the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositorySource {
    Flag,
    WorkflowConfig,
    AcceptanceExecutionConfig,
}

impl RepositorySource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Flag => "--repository",
            Self::WorkflowConfig => "[workflow] repository_root",
            Self::AcceptanceExecutionConfig => "[workflow.acceptance_execution] repository",
        }
    }
}

/// The repository the launch will ground every author and critic in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedRepository {
    /// Canonical absolute path of the checkout.
    pub(crate) root: PathBuf,
    /// `HEAD` of that checkout now, or `unborn`.
    pub(crate) base_commit: String,
    pub(crate) source: RepositorySource,
}

pub(crate) const NO_REPOSITORY_REMEDY: &str = "workflow decompose needs the code repository its authors read: pass --repository <PATH>, or set [workflow] repository_root = \"<PATH>\" in the project config (an existing [workflow.acceptance_execution] repository is used when neither is given); the working directory is never assumed";

/// Resolve the repository from the flag, then the workflow config, then the
/// acceptance-execution config. Relative paths resolve against `cwd`. The
/// result is an existing directory that is a git checkout, or an error that
/// names the source that pointed at it.
pub(crate) fn resolve_repository(
    cwd: &Path,
    flag: Option<&Path>,
    config: &ArchonConfig,
) -> Result<ResolvedRepository> {
    let (candidate, source) = if let Some(path) = flag {
        (path.to_path_buf(), RepositorySource::Flag)
    } else if let Some(path) = &config.workflow.repository_root {
        (path.clone(), RepositorySource::WorkflowConfig)
    } else if let Some(execution) = &config.workflow.acceptance_execution {
        (
            execution.repository.clone(),
            RepositorySource::AcceptanceExecutionConfig,
        )
    } else {
        return Err(anyhow!(NO_REPOSITORY_REMEDY));
    };
    let absolute = if candidate.is_absolute() {
        candidate.clone()
    } else {
        cwd.join(&candidate)
    };
    if !absolute.is_dir() {
        return Err(anyhow!(
            "{} names repository {} which is not an existing directory",
            source.label(),
            absolute.display()
        ));
    }
    let root = absolute.canonicalize().with_context(|| {
        format!(
            "canonicalizing repository {} named by {}",
            absolute.display(),
            source.label()
        )
    })?;
    if !is_git_checkout(&root) {
        return Err(anyhow!(
            "{} names repository {} which is not a git checkout (no .git and git reports no working tree); point it at the checkout the tasks describe",
            source.label(),
            root.display()
        ));
    }
    let base_commit = git_head(&root)
        .with_context(|| format!("reading HEAD of repository {}", root.display()))?;
    Ok(ResolvedRepository {
        root,
        base_commit,
        source,
    })
}

/// The record already under `task_root`, verified against the repository
/// this launch resolved. A different repository path is a refusal: the chain
/// frozen there was authored against another tree. A moved base commit is
/// returned for the caller to report.
pub(crate) fn verify_existing_record(
    task_root: &Path,
    resolved: &ResolvedRepository,
) -> Result<Option<RepositoryRecordV1>> {
    let Some(record) = read_repository_record(task_root)? else {
        return Ok(None);
    };
    let recorded = PathBuf::from(&record.repository_root);
    let recorded_canonical = recorded.canonicalize().unwrap_or(recorded);
    if recorded_canonical != resolved.root {
        return Err(anyhow!(
            "{} under {} records repository {} but this launch resolved {} from {}; a task set is decomposed against one repository — pass the recorded one or start a new task root",
            REPOSITORY_LOCK_FILE,
            task_root.display(),
            record.repository_root,
            resolved.root.display(),
            resolved.source.label()
        ));
    }
    Ok(Some(record))
}

/// Human-readable drift, when the recorded base is not the checkout's HEAD.
pub(crate) fn drift_text(
    record: &RepositoryRecordV1,
    resolved: &ResolvedRepository,
) -> Option<String> {
    (record.base_commit != resolved.base_commit).then(|| {
        format!(
            "repository {} was recorded at base commit {} by decomposition {} and is now at {}; the frozen chain was authored against the recorded base",
            resolved.root.display(),
            record.base_commit,
            record.decomposition_run_id,
            resolved.base_commit
        )
    })
}

/// Write the record for a task root that has none. Called once the run
/// exists, so the record names the run that grounded the task set.
pub(crate) fn record_launch(
    task_root: &Path,
    resolved: &ResolvedRepository,
    run_id: &str,
) -> Result<RepositoryRecordV1> {
    let record = RepositoryRecordV1 {
        schema_version: REPOSITORY_RECORD_SCHEMA_VERSION,
        repository_root: path_text(&resolved.root),
        base_commit: resolved.base_commit.clone(),
        decomposition_run_id: run_id.to_string(),
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    write_repository_record(task_root, &record).with_context(|| {
        format!(
            "writing {} under {}",
            REPOSITORY_LOCK_FILE,
            task_root.display()
        )
    })?;
    Ok(record)
}

/// The operator-log line naming the repository this run is grounded in. One
/// `key=value` line like the lifecycle markers; the path is JSON-quoted so a
/// space in it cannot split the record.
pub(crate) fn log_line(
    run_id: &str,
    resolved: &ResolvedRepository,
    record: &RepositoryRecordV1,
) -> String {
    let mut line = format!(
        "event=repository_grounded run_id={run_id} repository_root={} base_commit={} source={} recorded_base_commit={} recorded_by={}",
        serde_json::to_string(&path_text(&resolved.root)).unwrap_or_default(),
        resolved.base_commit,
        resolved.source.label().replace(' ', "_"),
        record.base_commit,
        record.decomposition_run_id,
    );
    if record.base_commit != resolved.base_commit {
        line.push_str(" drift=true");
    }
    line
}

pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
#[path = "workflow_decompose_repository_tests.rs"]
mod tests;
