//! Host evidence distinguishing project delivery from repository patch work.
use super::*;
use crate::v2::project_artifact_prompt::declared_project_artifacts;

/// Each declared project artifact as `(declared, project path, the branch's
/// copy)`. Batch G2: the branch never writes the project root; what it
/// delivers is its copy, which the host lands (`project_inputs_seed`), so
/// `before` is the project's state and `after` the copy's.
pub(super) struct ArtifactDelivery {
    paths: Vec<(String, PathBuf, PathBuf)>,
    before: BTreeMap<String, Option<String>>,
    /// Declared artifacts the branch has no copy of, and why: a defect of
    /// the declaration the host records, never the branch's missing work.
    refusals: Vec<(PathBuf, String)>,
    /// Declared artifacts the branch writes where they are (the run's own
    /// artifact directory).
    direct: Vec<PathBuf>,
}

fn digest(path: &Path) -> Option<String> {
    let meta = path.symlink_metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    std::fs::read(path)
        .ok()
        .map(|bytes| crate::task_set_contract::content_digest(&bytes))
}
fn is_non_empty_file(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}
fn same_tree(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
impl ArtifactDelivery {
    pub fn capture(
        prepared: &PreparedWorktreeBranch,
        store: &WorkflowV2ResultStore,
        run_root: &Path,
        stage_id: &str,
    ) -> Self {
        let context = crate::project_artifact_context_from_v2_root(store.root());
        let declared = declared_project_artifacts(
            &prepared.branch.input,
            &prepared.branch.call.options.required_artifacts,
            &context,
        );
        let seed: Option<crate::write_coordinator::project_inputs::SeedRecord> =
            crate::write_coordinator::project_inputs::read_json(
                &crate::write_coordinator::project_inputs::seed_path(
                    run_root,
                    stage_id,
                    &prepared.branch.id,
                ),
            );
        let worktree = &prepared.workspace.plan.isolated_root;
        let project = context.project_root.map(PathBuf::from);
        let project_is_repository = project
            .as_deref()
            .is_some_and(|project| same_tree(project, &prepared.coordinator_plan.canonical_root));
        let run_artifacts = run_root.join("artifacts");
        // Where the branch writes `absolute`, or why it cannot deliver it.
        let copy_of = |absolute: &Path| -> Result<PathBuf, String> {
            // The run's own artifact directory is the branch's to write.
            if absolute.starts_with(&run_artifacts) {
                return Ok(absolute.to_path_buf());
            }
            // Issue-226: a declared external data root the seed staged.
            let external = absolute.display().to_string();
            if let Some(copy) = seed.as_ref().and_then(|seed| seed.declared.get(&external)) {
                return Ok(copy.copy.clone());
            }
            let rel = project
                .as_deref()
                .and_then(|project| absolute.strip_prefix(project).ok())
                .ok_or_else(|| {
                    (seed.iter().flat_map(|seed| &seed.skipped))
                        .find(|(path, _)| *path == external)
                        .map_or_else(
                            || "outside the project root".to_string(),
                            |(_, why)| why.clone(),
                        )
                })?;
            let key = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            let Some(seed) = seed.as_ref() else {
                // No seed (no landing policy for this run): the worktree is
                // the copy when it is the project's; otherwise judged where
                // it is, as before, with no reason the host can state.
                return Ok(if project_is_repository {
                    worktree.join(rel)
                } else {
                    absolute.to_path_buf()
                });
            };
            if let Some(copy) = seed.declared.get(&key) {
                return Ok(copy.copy.clone());
            }
            if seed.inputs.iter().any(|input| rel.starts_with(input)) {
                return Ok(worktree.join(rel));
            }
            match seed.skipped.iter().find(|(path, _)| *path == key) {
                Some((_, why)) if why == super::project_inputs_seed::declared::CARRIED_BY_PATCH => {
                    Ok(worktree.join(rel))
                }
                Some((_, why)) => Err(why.clone()),
                None if project_is_repository => Ok(worktree.join(rel)),
                None => Ok(absolute.to_path_buf()),
            }
        };
        let mut refusals = Vec::new();
        let mut direct = Vec::new();
        let paths = declared
            .entries
            .into_iter()
            .map(|(raw, absolute)| {
                let absolute = PathBuf::from(absolute);
                let copy = match copy_of(&absolute) {
                    Ok(copy) => {
                        if copy == absolute && absolute.starts_with(&run_artifacts) {
                            direct.push(absolute.clone());
                        }
                        copy
                    }
                    // No copy the branch may write: judged where it is.
                    Err(why) => {
                        refusals.push((absolute.clone(), why));
                        absolute.clone()
                    }
                };
                (raw, absolute, copy)
            })
            .collect();
        let mut delivery = Self::from_copies(paths);
        delivery.refusals = refusals;
        delivery.direct = direct;
        delivery
    }
    /// Digest the declared artifacts NOW as the `before` state the later
    /// answers compare against; each judged where it is.
    #[cfg(test)]
    pub(super) fn from_paths(paths: Vec<(String, PathBuf)>) -> Self {
        Self::from_copies(
            paths
                .into_iter()
                .map(|(raw, path)| (raw, path.clone(), path))
                .collect(),
        )
    }
    fn from_copies(paths: Vec<(String, PathBuf, PathBuf)>) -> Self {
        let before = paths
            .iter()
            .map(|(raw, path, _)| (raw.clone(), digest(path)))
            .collect();
        Self {
            paths,
            before,
            refusals: Vec::new(),
            direct: Vec::new(),
        }
    }
    /// Each declared project artifact's project path and the branch's copy
    /// of it, where the branch has one: what its prompt names.
    pub(super) fn copies(&self) -> Vec<(PathBuf, PathBuf)> {
        self.paths
            .iter()
            .filter(|(_, path, copy)| path != copy || self.direct.contains(path))
            .map(|(_, path, copy)| (path.clone(), copy.clone()))
            .collect()
    }
    /// Declared artifacts no landing can deliver from this branch, and why.
    pub(super) fn refusals(&self) -> &[(PathBuf, String)] {
        &self.refusals
    }
    fn after(&self) -> BTreeMap<String, Option<String>> {
        self.paths
            .iter()
            .map(|(raw, _, copy)| (raw.clone(), digest(copy)))
            .collect()
    }
    fn changed_in(&self, after: &BTreeMap<String, Option<String>>) -> Vec<String> {
        after
            .iter()
            .filter(|(key, value)| self.before.get(*key) != Some(value))
            .map(|(key, _)| key.clone())
            .collect()
    }
    /// Declared artifacts whose digest differs from the one captured before
    /// the agent ran, answerable at any time after capture — the empty-patch
    /// gate reads it before `stamp` records the same answer (Issue-69). A
    /// declared artifact that did not exist before counts only once it
    /// exists with content: an empty file is a placeholder, not a delivery.
    pub fn changed_paths(&self) -> Vec<String> {
        let after = self.after();
        self.changed_in(&after)
            .into_iter()
            .filter(|key| {
                self.before.get(key).is_some_and(Option::is_some)
                    || self
                        .paths
                        .iter()
                        .any(|(raw, _, copy)| raw == key && is_non_empty_file(copy))
            })
            .collect()
    }
    pub fn stamp(&self, result: &mut WorkflowV2Result, repository_changed: bool) {
        if !result.data.is_object() {
            result.data = serde_json::json!({});
        }
        if self.paths.is_empty() {
            result.data["delivery"] = serde_json::json!({"kind":if repository_changed{"repository_patch"}else{"no_repository_change"},"repository_changed":repository_changed});
            return;
        }
        let after = self.after();
        let changed = self.changed_in(&after);
        result.data["delivery"] = serde_json::json!({"kind":"project_artifact","repository_changed":repository_changed,
            "changed_artifact_paths":changed,"before":self.before,"after":after});
        // Required-artifact validation remains authoritative; a receipt never
        // promotes failed output or claims a nonexistent file was delivered.
        if result.status == WorkflowV2Status::Accepted
            && !repository_changed
            && changed.is_empty()
            && self
                .paths
                .iter()
                .all(|(raw, _, _)| after.get(raw).is_some_and(Option::is_some))
        {
            result.status = WorkflowV2Status::Noop;
        }
    }
}
