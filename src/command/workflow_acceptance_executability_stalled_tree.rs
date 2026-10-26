use std::collections::{BTreeMap, BTreeSet};

use super::super::{Baseline, CheckEvidence, HostProbe, contract_digest, refs_for, repairs};
use archon_workflow::task_set_contract::AcceptanceContract;

impl HostProbe {
    /// Run checks on an explicitly selected commit using the hermetic tree
    /// runner shared by the baseline and failed-tree repair gates.
    pub(crate) async fn run_on_tree(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
        tree: &Baseline,
    ) -> std::result::Result<BTreeMap<String, CheckEvidence>, String> {
        let digest = contract_digest(contract)?;
        let refs = refs_for(contract, &digest, ids);
        let run = repairs::tree_results(self, tree, contract, &digest, &refs, None, false).await;
        Ok(run.results)
    }
}
