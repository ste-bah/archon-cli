//! Diagnostic records shared by all publication paths.
use super::workflow_task_set::coverage_gate;
use anyhow::{Context, Result};
use archon_workflow::baseline_results::{BaselineResults, results_digest};
use archon_workflow::repository_record::{RepositoryTree, read_repository_record};
use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceLock};
use archon_workflow::task_skeleton::TaskSkeleton;
use archon_workflow::v2::requirement_classes::ClassificationError;
use archon_workflow::v2::requirement_classes::{RequirementClassification, classify_checked};
use std::path::Path;
#[path = "workflow_task_set_landing_verified.rs"]
mod landing_verified;
#[path = "workflow_task_set_wiring_roots.rs"]
mod wiring_roots;

pub(crate) fn baseline_results(
    probed: &coverage_gate::PreImplementation,
    contract: &AcceptanceContract,
) -> BaselineResults {
    BaselineResults::observed(
        probed.baseline.as_ref().map(|runs| runs.commit.clone()),
        &probed.baseline_outcomes,
        contract,
    )
}

pub(crate) fn carry_baseline_results(
    tasks_root: &Path,
    commit: Option<String>,
    contract: &AcceptanceContract,
) -> Result<BaselineResults> {
    let path = tasks_root.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let lock: AcceptanceLock =
                serde_json::from_slice(&bytes).context("parsing predecessor acceptance lock")?;
            let commit = commit.or_else(|| lock.baseline_commit.clone());
            Ok(BaselineResults::retained(&lock, commit, contract))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(commit.into()),
        Err(error) => Err(error).context("reading predecessor acceptance lock"),
    }
}

pub(crate) fn classification(
    tasks_root: &Path,
    project_root: &Path,
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    lock: &AcceptanceLock,
) -> RequirementClassification {
    let mut record = RequirementClassification {
        schema_version: 2,
        base_snapshot: None,
        baseline_commit: lock.baseline_commit.clone(),
        acceptance_digest: lock.digest.clone(),
        baseline_results_digest: results_digest(&lock.baseline_results),
        error: None,
        entries: Vec::new(),
    };
    let tree = (|| -> std::result::Result<RepositoryTree, ClassificationError> {
        let mut repository = read_repository_record(tasks_root)
            .map_err(|error| match error {
                archon_workflow::WorkflowError::Io { .. } => ClassificationError::Transient {
                    transient: error.to_string(),
                },
                _ => ClassificationError::from("repository_record_unreadable"),
            })?
            .ok_or("repository_record_absent")?;
        repository.base_commit = lock
            .baseline_commit
            .clone()
            .filter(|commit| !commit.trim().is_empty())
            .unwrap_or_else(|| archon_workflow::repository_record::UNBORN_BASE_COMMIT.into());
        if lock
            .baseline_commit
            .as_ref()
            .is_none_or(|c| c.trim().is_empty())
        {
            record.error = Some("baseline_commit_absent".into());
        }
        if !Path::new(&repository.repository_root).is_dir() {
            return Err("repository_root_absent".into());
        }
        RepositoryTree::load(&repository).map_err(|error| match error {
            archon_workflow::WorkflowError::Io { .. } => ClassificationError::Transient {
                transient: error.to_string(),
            },
            _ => ClassificationError::from("baseline_tree_unavailable"),
        })
    })();
    match tree {
        Ok(tree) => {
            let paths = skeleton
                .tasks
                .iter()
                .flat_map(|task| &task.deliverable_contracts)
                .map(|d| d.artifact_path.as_str());
            match archon_workflow::base_snapshot::snapshot_deliverables(&tree, project_root, paths)
            {
                Ok(mut snapshot) => {
                    landing_verified::mark(project_root, tasks_root, tree.root(), &mut snapshot);
                    record.entries =
                        classify_checked(skeleton, contract, &lock.baseline_results, |path| {
                            let key = archon_workflow::base_snapshot::snapshot_key(&tree, path);
                            let entry = snapshot
                                .entries
                                .get(&key)
                                .ok_or_else(|| format!("snapshot entry missing: {key}"))?;
                            Ok((
                                matches!(
                                    entry,
                                    archon_workflow::base_snapshot::SnapshotEntry::Tracked { .. }
                                ) || snapshot.landing_verified.contains_key(&key),
                                matches!(
                                    entry,
                                    archon_workflow::base_snapshot::SnapshotEntry::Unverified { .. }
                                ),
                            ))
                        });
                    for entry in &mut record.entries {
                        for d in &mut entry.deliverables {
                            d.landing_verified = snapshot.landing_verified.contains_key(
                                &archon_workflow::base_snapshot::snapshot_path_key(
                                    tree.root(),
                                    &d.artifact_path,
                                ),
                            );
                        }
                    }
                    wiring_roots::mark(&mut record.entries, &snapshot, tree.root());
                    record.base_snapshot = Some(snapshot);
                }
                Err(error) => {
                    record.entries =
                        classify_checked(skeleton, contract, &lock.baseline_results, |_| {
                            Err(error.clone())
                        });
                    record.error = Some(error.into());
                }
            }
        }
        Err(error) => {
            record.entries = classify_checked(skeleton, contract, &lock.baseline_results, |_| {
                Err(error.to_string())
            });
            record.error = Some(error);
        }
    }
    record
}

pub(crate) fn classification_from_lock(
    tasks_root: &Path,
    project_root: &Path,
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
) -> RequirementClassification {
    let path = tasks_root.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
    match std::fs::read(path)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| Ok(serde_json::from_slice::<AcceptanceLock>(&bytes)?))
    {
        Ok(lock) => classification(tasks_root, project_root, skeleton, contract, &lock),
        Err(_) => RequirementClassification {
            schema_version: 2,
            base_snapshot: None,
            baseline_commit: None,
            acceptance_digest: skeleton.acceptance_digest.clone(),
            baseline_results_digest: results_digest(&Default::default()),
            error: Some("acceptance_lock_unreadable".into()),
            entries: Vec::new(),
        },
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    use archon_core::config::GateMode;
    use archon_workflow::repository_record::write_repository_record;
    use archon_workflow::task_set_contract::TrustedCwd;

    pub(super) fn skeleton() -> TaskSkeleton {
        serde_json::from_value(serde_json::json!({"schema_version":1,"acceptance_digest":"anchor","tasks":[{"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["AC-X-001"],"deliverable_contracts":[{"kind":"output","artifact_path":"feature.txt","min_instances":1,"implements":["AC-X-001"]}]}]})).unwrap()
    }

    #[test]
    fn classification_errors_never_fall_back_to_head() {
        let trees = trees(&[("AC-X-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
        let lock_path = trees
            .set
            .tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
        let mut lock: AcceptanceLock =
            serde_json::from_slice(&std::fs::read(lock_path).unwrap()).unwrap();
        lock.baseline_commit = Some(trees.base.clone());
        let mut record = read_repository_record(&trees.set.tasks).unwrap().unwrap();
        record.base_commit = trees.head.clone();
        write_repository_record(&trees.set.tasks, &record).unwrap();
        let good = classification(
            &trees.set.tasks,
            trees.set.project.path(),
            &skeleton(),
            &trees.contract(),
            &lock,
        );
        assert!(good.error.is_none(), "{good:?}");
        assert_eq!(
            good.entries[0].class,
            archon_workflow::v2::requirement_classes::RequirementClass::Missing
        );
        assert!(good.entries[0].deliverables[0].live_only);
        assert!(!good.entries[0].deliverables[0].at_base);
        lock.baseline_commit = Some("invalid-commit".into());
        let bad = classification(
            &trees.set.tasks,
            trees.set.project.path(),
            &skeleton(),
            &trees.contract(),
            &lock,
        );
        assert_eq!(bad.error.as_deref(), Some("baseline_tree_unavailable"));
        assert_eq!(bad.entries.len(), 1);
        lock.baseline_commit = Some(archon_workflow::repository_record::UNBORN_BASE_COMMIT.into());
        let unborn = classification(
            &trees.set.tasks,
            trees.set.project.path(),
            &skeleton(),
            &trees.contract(),
            &lock,
        );
        assert!(unborn.error.is_none());
        assert_eq!(
            unborn.entries[0].class,
            archon_workflow::v2::requirement_classes::RequirementClass::Missing
        );
        assert!(unborn.base_snapshot.is_some());
        lock.baseline_commit = None;
        let missing = classification(
            &trees.set.tasks,
            trees.set.project.path(),
            &skeleton(),
            &trees.contract(),
            &lock,
        );
        assert_eq!(missing.error.as_deref(), Some("baseline_commit_absent"));
        assert_eq!(
            missing.entries[0].class,
            archon_workflow::v2::requirement_classes::RequirementClass::Missing
        );
        std::fs::remove_file(archon_workflow::repository_record::repository_record_path(
            &trees.set.tasks,
        ))
        .unwrap();
        let missing = classification(
            &trees.set.tasks,
            trees.set.project.path(),
            &skeleton(),
            &trees.contract(),
            &lock,
        );
        assert_eq!(missing.error.as_deref(), Some("repository_record_absent"));
        assert_eq!(missing.entries.len(), 1);
    }
    #[test]
    fn normal_empty_and_really_unborn_baselines_classify_every_id() {
        let trees = trees(&[("AC-X-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
        let mut lock: AcceptanceLock = serde_json::from_slice(
            &std::fs::read(
                trees
                    .set
                    .tasks
                    .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
            )
            .unwrap(),
        )
        .unwrap();
        let repository = read_repository_record(&trees.set.tasks).unwrap().unwrap();
        for baseline in [trees.base.as_str(), "", "unborn"] {
            if baseline == "unborn" {
                let output = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&repository.repository_root)
                    .args(["update-ref", "-d", "HEAD"])
                    .output()
                    .unwrap();
                assert!(output.status.success());
                assert_eq!(
                    archon_workflow::repository_record::git_head(Path::new(
                        &repository.repository_root
                    ))
                    .unwrap(),
                    "unborn"
                );
            }
            lock.baseline_commit = Some(baseline.into());
            let result = classification(
                &trees.set.tasks,
                trees.set.project.path(),
                &skeleton(),
                &trees.contract(),
                &lock,
            );
            assert_eq!(result.entries.len(), 1, "{result:?}");
            assert_eq!(
                result.entries[0].class,
                archon_workflow::v2::requirement_classes::RequirementClass::Missing
            );
            assert!(result.entries[0].reason.is_some());
            assert!(result.base_snapshot.is_some());
            if baseline.is_empty() {
                assert_eq!(result.error.as_deref(), Some("baseline_commit_absent"));
            } else {
                assert!(result.error.is_none(), "{result:?}");
            }
        }
    }

    #[test]
    fn s4a_rejudge_retains_only_unchanged_checks_on_the_same_commit() {
        use archon_workflow::baseline_results::BaselineOutcome;
        let trees = trees(&[("AC-X-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
        let mut contract = trees.contract();
        let path = trees
            .set
            .tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
        let mut lock: AcceptanceLock =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        lock.baseline_commit = Some(trees.base.clone());
        lock.baseline_results = BaselineResults::observed(
            lock.baseline_commit.clone(),
            &[("AC-X-001".into(), BaselineOutcome::Failed)].into(),
            &contract,
        )
        .outcomes;
        std::fs::write(&path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        let prepare = |contract: &AcceptanceContract, commit: &str| {
            let prepared = crate::command::workflow_task_set::findings::prepare_from_judged(
                trees.set.project.path(),
                &trees.set.tasks,
                &trees.set.prd,
                GateMode::Observe,
                contract,
                Some(commit.into()),
                None,
            )
            .unwrap();
            let (_, parts) = prepared.into_staged_parts();
            let (_, bytes) = parts
                .into_iter()
                .find(|(name, _)| name == archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE)
                .unwrap();
            serde_json::from_slice::<AcceptanceLock>(&bytes).unwrap()
        };
        assert_eq!(
            prepare(&contract, &trees.base).baseline_results,
            lock.baseline_results
        );
        assert!(prepare(&contract, &trees.head).baseline_results.is_empty());
        contract.acceptance[0].check =
            archon_workflow::task_set_contract::AcceptanceCheck::Command {
                command: "test -s feature.txt".into(),
                cwd: TrustedCwd::RepoRoot,
            };
        assert!(prepare(&contract, &trees.base).baseline_results.is_empty());
    }
}

#[cfg(all(test, unix))]
mod presence_tests {
    use super::*;
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    use archon_workflow::task_set_contract::TrustedCwd;
    use archon_workflow::v2::requirement_classes::RequirementClass;

    #[test]
    fn outside_deliverable_is_unverified_and_missing() {
        let trees = trees(&[("AC-X-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
        let outside = tempfile::tempdir().unwrap();
        let path = outside
            .path()
            .join("artifact")
            .to_string_lossy()
            .to_string();
        let skeleton: TaskSkeleton = serde_json::from_value(serde_json::json!({
            "schema_version":1,"acceptance_digest":"anchor","tasks":[{
                "task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["AC-X-001"],
                "deliverable_contracts":[{"kind":"output","artifact_path":path,"min_instances":1,"implements":["AC-X-001"]}]
            }]
        })).unwrap();
        let mut lock: AcceptanceLock = serde_json::from_slice(
            &std::fs::read(
                trees
                    .set
                    .tasks
                    .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
            )
            .unwrap(),
        )
        .unwrap();
        lock.baseline_commit = Some(trees.base.clone());
        let record = classification(
            &trees.set.tasks,
            trees.set.project.path(),
            &skeleton,
            &trees.contract(),
            &lock,
        );
        assert!(record.error.is_none(), "{record:?}");
        assert_eq!(record.entries[0].class, RequirementClass::Missing);
        assert_eq!(
            record.entries[0].reason,
            Some("deliverable_unverified_at_base".into())
        );
    }
    #[test]
    fn missing_recorded_repository_has_its_own_error_code() {
        let trees = trees(&[("AC-X-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
        let lock: AcceptanceLock = serde_json::from_slice(
            &std::fs::read(
                trees
                    .set
                    .tasks
                    .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
            )
            .unwrap(),
        )
        .unwrap();
        let mut repository = read_repository_record(&trees.set.tasks).unwrap().unwrap();
        repository.repository_root = trees
            .set
            .project
            .path()
            .join("absent-repository")
            .display()
            .to_string();
        archon_workflow::repository_record::write_repository_record(&trees.set.tasks, &repository)
            .unwrap();
        let result = classification(
            &trees.set.tasks,
            trees.set.project.path(),
            &super::tests::skeleton(),
            &trees.contract(),
            &lock,
        );
        assert_eq!(result.error.as_deref(), Some("repository_root_absent"));
    }
}
