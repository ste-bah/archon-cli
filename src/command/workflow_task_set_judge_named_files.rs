use super::*;
use sha2::{Digest, Sha256};
use std::path::Path;

#[path = "workflow_task_set_judge_named_files_repository.rs"]
mod repository;
#[path = "workflow_task_set_judge_named_files_text.rs"]
mod text;
use repository::{repository_regular_blob, repository_regular_blob_size, repository_tree_entry};

#[derive(Default)]
pub(super) struct PrefetchSourceCache {
    content: BTreeMap<(PathBuf, String), Vec<u8>>,
    repository_path_digest: BTreeMap<(PathBuf, String, String), String>,
}

#[cfg(test)]
pub(super) fn named_files(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> Vec<serde_json::Value> {
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    named_files_with_budget(criterion, roots, &mut remaining)
}

/// Resolve the named source for one check while spending a shared prompt-batch
/// budget. Callers rendering a batch pass the same `remaining` value to each
/// check in prompt order.
pub(super) fn named_files_with_budget(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
    remaining: &mut usize,
) -> Vec<serde_json::Value> {
    named_files_for_inputs_with_budget(criterion, roots, remaining, &[])
}

pub(super) fn named_files_for_inputs_with_budget(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
    remaining: &mut usize,
    extra_inputs: &[String],
) -> Vec<serde_json::Value> {
    let mut cache = PrefetchSourceCache::default();
    named_files_for_inputs_with_budget_cached(criterion, roots, remaining, extra_inputs, &mut cache)
}

pub(super) fn named_files_for_inputs_with_budget_cached(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
    remaining: &mut usize,
    extra_inputs: &[String],
    cache: &mut PrefetchSourceCache,
) -> Vec<serde_json::Value> {
    resolve_named_files(criterion, roots, extra_inputs, cache, Some(*remaining))
        .into_iter()
        .map(|file| match file.bytes {
            _ if file.status == "not_judgeable" => serde_json::json!({
                "path": file.path, "status": file.status,
                "detail": "The host found this file among the words of the check, but its first block contains NUL bytes. Its binary content is not judgeable text; if it decides the check, its assertions are unknown."
            }),
            Some(bytes) => supplied(&file.path, &bytes, remaining),
            None if file.status == "over_budget" => over_budget(&file.path),
            // Discovery finds path-shaped text, not necessarily file operands.
            None if file.status == "absent" => serde_json::json!({
                "path": file.path,
                "status": file.status,
                "detail": "The host found no file at this path. It may be a pattern or text argument, not a file; this alone is not evidence that behaviour is missing."
            }),
            None => serde_json::json!({"path": file.path, "status": file.status}),
        })
        .collect()
}

struct ResolvedNamedFile {
    path: String,
    bytes: Option<Vec<u8>>,
    status: &'static str,
}

impl ResolvedNamedFile {
    fn supplied(path: String, bytes: Vec<u8>, remaining: &mut Option<usize>) -> Self {
        if !text::is_text(&bytes) {
            // Keep complete raw bytes for reuse identity, never for the prompt budget.
            return Self {
                path,
                bytes: Some(bytes),
                status: "not_judgeable",
            };
        }
        if let Some(left) = remaining {
            if bytes.len() > *left {
                return Self::unavailable(path, "over_budget");
            }
            *left -= bytes.len();
        }
        Self {
            path,
            bytes: Some(bytes),
            status: "supplied",
        }
    }

    fn unavailable(path: String, status: &'static str) -> Self {
        Self {
            path,
            bytes: None,
            status,
        }
    }
}

/// Resolve prompt bodies only when they fit the remaining shared budget.
/// Full-content judgment reuse passes no budget and hashes complete raw bytes.
fn resolve_named_files(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
    extra_inputs: &[String],
    cache: &mut PrefetchSourceCache,
    mut remaining: Option<usize>,
) -> Vec<ResolvedNamedFile> {
    let root = match &criterion.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command {
            cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
            ..
        } => roots.repository.as_deref(),
        _ => roots.project.as_deref(),
    };
    let Some(root) = root.and_then(|path| path.canonicalize().ok()) else {
        return Vec::new();
    };
    let project = roots
        .project
        .as_deref()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| root.clone());
    let repository = roots
        .repository
        .as_deref()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| root.clone());
    let task_root = roots
        .task_root
        .as_deref()
        .and_then(|path| path.canonicalize().ok());
    let project_input_excludes = task_root.as_deref().and_then(|task_root| {
        match crate::command::acceptance_scratch_policy::capture(&project, task_root) {
            Ok(Some(binding)) => Some(binding.policy.project_input_excludes),
            Ok(None) => Some(Vec::new()),
            Err(_) => None,
        }
    });
    let task_root_relative = task_root
        .as_deref()
        .and_then(|task_root| task_root.strip_prefix(&project).ok());
    let nested_repository =
        archon_workflow::acceptance_scratch::nested_repository(&project, &repository)
            .filter(|nested| !nested.as_os_str().is_empty());
    let mut inputs = crate::command::workflow_task_set::executability::named_inputs(
        criterion,
        &project,
        &repository,
    );
    inputs.extend(extra_inputs.iter().cloned());
    inputs.sort();
    inputs.dedup();
    inputs
        .into_iter()
        .map(|relative| {
            let Some(path) = normalize_named_path(Path::new(&relative)) else {
                return ResolvedNamedFile::unavailable(
                    archon_shell::paths::stored_text(Path::new(&relative)),
                    "not_supplied",
                );
            };
            let absolute = root.join(&path);
            let path_text = archon_shell::paths::stored_text(&path);
            if matches!(
                criterion.check,
                archon_workflow::task_set_contract::AcceptanceCheck::Command {
                    cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
                    ..
                }
            ) {
                let Some(commit) = roots.repository_commit.as_deref() else {
                    let status = repository_missing_status(&repository, None, &path_text, &path);
                    return ResolvedNamedFile::unavailable(path_text, status);
                };
                let stored_path = archon_shell::paths::stored_text(&path);
                if let Some(left) = remaining {
                    let Some(size) =
                        repository_regular_blob_size(&repository, commit, &stored_path)
                    else {
                        let status = repository_missing_status(
                            &repository,
                            Some(commit),
                            &stored_path,
                            &path,
                        );
                        return ResolvedNamedFile::unavailable(path_text, status);
                    };
                    match text::repository_is_text(&repository, commit, &stored_path) {
                        Ok(false) => {
                            return ResolvedNamedFile::unavailable(path_text, "not_judgeable");
                        }
                        Err(_) => return ResolvedNamedFile::unavailable(path_text, "not_supplied"),
                        Ok(true) => {}
                    }
                    if size > left as u64 {
                        return ResolvedNamedFile::unavailable(path_text, "over_budget");
                    }
                }
                let resolved_path = repository.join(&path);
                let lookup = (repository.clone(), commit.to_string(), stored_path.clone());
                let cached = cache
                    .repository_path_digest
                    .get(&lookup)
                    .and_then(|digest| cache.content.get(&(resolved_path.clone(), digest.clone())))
                    .cloned();
                let content_bytes = cached.or_else(|| {
                    let bytes = repository_regular_blob(&repository, commit, &stored_path)?;
                    let digest = format!("{:x}", Sha256::digest(&bytes));
                    cache
                        .content
                        .insert((resolved_path.clone(), digest.clone()), bytes.clone());
                    cache.repository_path_digest.insert(lookup, digest);
                    Some(bytes)
                });
                let Some(content_bytes) = content_bytes else {
                    let status =
                        repository_missing_status(&repository, Some(commit), &stored_path, &path);
                    return ResolvedNamedFile::unavailable(path_text, status);
                };
                return ResolvedNamedFile::supplied(path_text, content_bytes, &mut remaining);
            } else {
                match absolute.symlink_metadata() {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return ResolvedNamedFile::unavailable(path_text, "absent");
                    }
                    Err(_) => {
                        return ResolvedNamedFile::unavailable(path_text, "not_supplied");
                    }
                    Ok(_) => {}
                }
                let Ok(relative) = absolute.strip_prefix(&project) else {
                    return ResolvedNamedFile::unavailable(path_text, "not_supplied");
                };
                if project_input_excludes.as_ref().is_none_or(|excludes| {
                    archon_workflow::acceptance_scratch::acceptance_input_excluded(
                        relative,
                        excludes,
                        task_root_relative,
                    )
                }) || nested_repository
                    .as_ref()
                    .is_some_and(|nested| relative.starts_with(nested))
                    || relative
                        .components()
                        .any(|component| component.as_os_str() == ".git")
                    || !project_file_is_copied(&project, relative)
                {
                    return ResolvedNamedFile::unavailable(path_text, "not_supplied");
                }
            }
            let canonical = match absolute.canonicalize() {
                Ok(canonical) => canonical,
                Err(_) => {
                    return ResolvedNamedFile::unavailable(path_text, "not_supplied");
                }
            };
            if !canonical.starts_with(&root) || !canonical.is_file() {
                return ResolvedNamedFile::unavailable(path_text, "not_supplied");
            }
            let Ok(metadata) = canonical.metadata() else {
                return ResolvedNamedFile::unavailable(path_text, "not_supplied");
            };
            if remaining.is_some() {
                match text::file_is_text(&canonical) {
                    Ok(false) => return ResolvedNamedFile::unavailable(path_text, "not_judgeable"),
                    Err(_) => return ResolvedNamedFile::unavailable(path_text, "not_supplied"),
                    Ok(true) => {}
                }
            }
            if remaining.is_some_and(|left| metadata.len() > left as u64) {
                return ResolvedNamedFile::unavailable(path_text, "over_budget");
            }
            // A file the host cannot read is reported, never dropped.
            #[cfg(test)]
            budget_tests::record_body_read();
            let Ok(bytes) = std::fs::read(&canonical) else {
                return ResolvedNamedFile::unavailable(path_text, "not_supplied");
            };
            ResolvedNamedFile::supplied(path_text, bytes, &mut remaining)
        })
        .collect()
}

#[path = "workflow_task_set_judge_duplicate.rs"]
mod duplicate;
pub(super) use duplicate::{exists_in_base, same_check_in_base, same_path};

pub(super) fn over_budget(path: &str) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "status": "not_supplied",
        "detail": "[not supplied: larger than the judged-source byte limit left for this batch]"
    })
}

/// The entry for a named file whose `bytes` the host read: supplied whole
/// within the batch's `remaining` budget. A file the budget would cut is
/// not supplied and carries no partial content: a not_supplied input's
/// assertions are unknown to the judge, so none of them is shown, and it
/// spends none of the budget.
fn supplied(path: &str, bytes: &[u8], remaining: &mut usize) -> serde_json::Value {
    if bytes.len() > *remaining {
        return over_budget(path);
    }
    *remaining -= bytes.len();
    let content =
        archon_observability::redaction::redact_secret_values(&String::from_utf8_lossy(bytes));
    serde_json::json!({"path": path, "status": "supplied", "content": content})
}

fn repository_missing_status(
    repository: &Path,
    commit: Option<&str>,
    stored_path: &str,
    relative: &Path,
) -> &'static str {
    let working_path = repository.join(relative);
    if commit.is_some_and(|commit| repository_tree_entry(repository, commit, stored_path).is_some())
        || working_path.symlink_metadata().is_ok()
        || working_path.exists()
    {
        "not_supplied"
    } else {
        "absent"
    }
}

/// Digest the exact path/content list the judge prompt will receive.
#[cfg(test)]
pub(in crate::command::workflow_task_set) fn named_files_digest(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> String {
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    named_files_digest_with_budget(criterion, roots, &mut remaining)
}

/// Digest each named input's complete raw content for judgment reuse. Prompt
/// redaction and the batch source budget affect only what a new judge sees.
pub(in crate::command::workflow_task_set) fn named_files_content_digest(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> String {
    let resolved_roots = super::each::with_commit(roots);
    let mut cache = PrefetchSourceCache::default();
    let files = resolve_named_files(criterion, &resolved_roots, &[], &mut cache, None)
        .into_iter()
        .map(|file| match file.bytes {
            Some(bytes) => serde_json::json!({
                "path": file.path,
                "status": file.status,
                "content_digest": format!("{:x}", Sha256::digest(bytes))
            }),
            None => serde_json::json!({"path": file.path, "status": file.status}),
        })
        .collect::<Vec<_>>();
    named_files_digest_for_list(&files)
}

/// The complete byte size of each named file of `criterion` the host can
/// read, in path order, under `roots` with its commit already resolved.
pub(super) fn named_file_sizes(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> Vec<(String, usize)> {
    let mut cache = PrefetchSourceCache::default();
    resolve_named_files(criterion, roots, &[], &mut cache, Some(usize::MAX))
        .into_iter()
        .filter(|file| file.status == "supplied")
        .filter_map(|file| Some((file.path, file.bytes?.len())))
        .collect()
}

#[cfg(test)]
pub(in crate::command::workflow_task_set) fn named_files_digest_with_budget(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
    remaining: &mut usize,
) -> String {
    let resolved = super::each::with_commit(roots);
    named_files_digest_for_list(&named_files_with_budget(criterion, &resolved, remaining))
}

pub(in crate::command::workflow_task_set) fn named_files_digest_for_list(
    files: &[serde_json::Value],
) -> String {
    content_digest(
        &serde_json::to_vec(files).expect("named judge files are serializable JSON values"),
    )
}

fn normalize_named_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn project_file_is_copied(project_root: &Path, relative: &Path) -> bool {
    use std::path::Component;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let Ok(root) = project_root.canonicalize() else {
        return false;
    };
    let candidate = root.join(relative);
    if candidate.canonicalize().ok().as_deref() != Some(candidate.as_path()) {
        return false;
    }
    let mut current = project_root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(std::path::Component::Normal(name)) = components.next() {
        current.push(name);
        let Ok(metadata) = current.symlink_metadata() else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return false;
            }
        } else if !metadata.is_file() {
            return false;
        }
    }
    !relative.as_os_str().is_empty()
}

#[cfg(test)]
#[path = "workflow_task_set_judge_named_files_budget_tests.rs"]
pub(crate) mod budget_tests;

pub(super) fn coverage_claim(
    check: &mut serde_json::Value,
    criterion: &AcceptanceCriterion,
    roots: &super::JudgeRoots,
) {
    // The requirement ids the check answers for are part of what it
    // claims: a check covering a requirement it cannot fail on is
    // refuted like one that cannot fail on its criterion.
    if !criterion.covers.is_empty() {
        check["covers"] = serde_json::json!(criterion.covers);
        let covers_text: BTreeMap<_, _> = criterion
            .covers
            .iter()
            .map(|id| {
                let text = roots.covered_texts.get(id.trim());
                (id, text.map(String::as_str).unwrap_or_default())
            })
            .collect();
        check["covers_text"] = serde_json::json!(covers_text);
    }
}
