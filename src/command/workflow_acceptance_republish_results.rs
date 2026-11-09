use super::*;
use crate::command::workflow_task_set::executability::ExecutabilityProbe;
pub(super) fn observed_for_republish(
    tasks: &Path,
    probe: &dyn ExecutabilityProbe,
    contract: &AcceptanceContract,
    published: &BTreeSet<String>,
) -> Result<archon_workflow::baseline_results::BaselineResults> {
    let lock: AcceptanceLock = serde_json::from_slice(&std::fs::read(
        tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
    )?)?;
    let mut results = findings::carry_baseline_results(tasks, probe.baseline_commit(), contract)?;
    results.outcomes.retain(|id, _| !published.contains(id));
    let observations = probe.take_baseline_observations();
    if probe.baseline_commit() == lock.baseline_commit {
        let observed = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .filter_map(|entry| {
                let key = (
                    entry.id.clone(),
                    archon_workflow::baseline_results::check_digest(&entry.check),
                );
                observations
                    .get(&key)
                    .map(|outcome| (entry.id.clone(), *outcome))
            })
            .collect();
        let mut fresh = archon_workflow::baseline_results::BaselineResults::observed(
            probe.baseline_commit(),
            &observed,
            contract,
        );
        fresh.outcomes.retain(|id, _| published.contains(id));
        results.outcomes.extend(fresh.outcomes);
    }
    Ok(results)
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};
    fn set() -> super::super::test_fixture::FrozenSet {
        let set = super::super::test_fixture::frozen_set_proven(&[("AC-F-001", "test -d .", true)]);
        let path = set
            .tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
        let mut lock: AcceptanceLock =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        lock.baseline_commit = set.probe.baseline_commit();
        lock.baseline_results = archon_workflow::baseline_results::BaselineResults::observed(
            lock.baseline_commit.clone(),
            &[(
                "AC-F-001".into(),
                archon_workflow::baseline_results::BaselineOutcome::Passed,
            )]
            .into(),
            &set.contract(),
        )
        .outcomes;
        std::fs::write(path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        set
    }
    #[tokio::test]
    async fn published_observation_uses_the_new_check_digest() {
        let set = set();
        let mut contract = set.contract();
        contract.acceptance[0].check = AcceptanceCheck::Command {
            command: "test -d . && test -d .".into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        let ids = ["AC-F-001".into()].into();
        set.probe.script_defects(&contract, &ids).await;
        let result = observed_for_republish(&set.tasks, &set.probe, &contract, &ids).unwrap();
        let record = &result.outcomes["AC-F-001"];
        assert_eq!(
            record.check_digest,
            archon_workflow::baseline_results::check_digest(&contract.acceptance[0].check)
        );
        assert_eq!(
            record.outcome,
            archon_workflow::baseline_results::BaselineOutcome::Passed
        );
    }
    #[tokio::test]
    async fn failed_observation_survives_passability_drain() {
        let set = set();
        let mut contract = set.contract();
        contract.acceptance[0].check = AcceptanceCheck::Command {
            command: "test -f absent_product_file".into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        let ids = ["AC-F-001".into()].into();
        set.probe.script_defects(&contract, &ids).await;
        assert!(
            set.probe
                .take_baseline_runs()
                .unwrap()
                .failures
                .contains_key("AC-F-001")
        );
        let result = observed_for_republish(&set.tasks, &set.probe, &contract, &ids).unwrap();
        assert_eq!(
            result.outcomes["AC-F-001"].outcome,
            archon_workflow::baseline_results::BaselineOutcome::Failed
        );
    }
    #[tokio::test]
    async fn observed_original_is_not_a_cached_repair_observation() {
        let set = set();
        let original = set.contract();
        let ids = ["AC-F-001".into()].into();
        set.probe.script_defects(&original, &ids).await;
        let mut repaired = original;
        repaired.acceptance[0].check = AcceptanceCheck::Command {
            command: "test -d . && test -d .".into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        let result = observed_for_republish(&set.tasks, &set.probe, &repaired, &ids).unwrap();
        assert!(result.outcomes.is_empty());
    }
    #[test]
    fn published_without_observation_is_absent() {
        let set = set();
        let contract = set.contract();
        let ids = ["AC-F-001".into()].into();
        let result = observed_for_republish(&set.tasks, &set.probe, &contract, &ids).unwrap();
        assert!(result.outcomes.is_empty());
    }
    #[test]
    fn unpublished_changed_digest_is_dropped() {
        let set = set();
        let mut contract = set.contract();
        contract.acceptance[0].check = AcceptanceCheck::Command {
            command: "test -d . && test -d .".into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        let result =
            observed_for_republish(&set.tasks, &set.probe, &contract, &BTreeSet::new()).unwrap();
        assert!(result.outcomes.is_empty());
    }
}
