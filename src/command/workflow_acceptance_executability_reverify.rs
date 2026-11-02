//! Frozen-check observation using the probe's typed site failures.
use super::*;
impl HostProbe {
    /// Run frozen accepted checks against the probe's current hermetic site.
    /// Reverification deliberately does not reuse the pre-implementation run:
    /// it needs the finished tree and the copy layout that actually ran it.
    pub(crate) async fn run_reverify_copy(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> std::result::Result<
        (
            Vec<CheckEvidence>,
            Option<String>,
            BTreeMap<String, String>,
            BTreeMap<String, String>,
        ),
        String,
    > {
        let digest = contract_digest(contract)?;
        let refs = refs_for(contract, &digest, ids);
        let Some(commit) = self.site_commit() else {
            let reason = match &self.site {
                Site::Unavailable(reason) => reason.0.clone(),
                _ => format!("{} has no commit to copy", self.repository.display()),
            };
            for reference in &refs {
                let cause = match &self.site {
                    Site::Unavailable(why) => UnprovenCause::from_class(why.1),
                    _ => UnprovenCause::PermanentHost,
                };
                self.unproven_as(&reference.acceptance_id, reason.clone(), cause);
            }
            return Ok((
                Vec::new(),
                None,
                refs.into_iter()
                    .map(|reference| (reference.acceptance_id, reason.clone()))
                    .collect(),
                BTreeMap::new(),
            ));
        };
        let tree = Baseline {
            commit: commit.clone(),
            repository: self.repository.clone(),
        };
        let run = repairs::tree_results(self, &tree, contract, &digest, &refs, None, true).await;
        for reference in &refs {
            if let Some(why) = run.unrun(&reference.acceptance_id) {
                let class = if run
                    .results
                    .get(&reference.acceptance_id)
                    .is_some_and(|item| {
                        item.operational_error.as_deref()
                            == Some(archon_workflow::acceptance_scratch::CHECK_TIMED_OUT)
                    }) {
                    crate::command::workflow_host_fault::Class::Transient
                } else {
                    run.fault_class
                };
                self.unproven_as(
                    &reference.acceptance_id,
                    why,
                    UnprovenCause::from_class(class),
                );
            }
        }
        let missing = refs
            .iter()
            .filter_map(|reference| {
                run.unrun(&reference.acceptance_id)
                    .map(|why| (reference.acceptance_id.clone(), why))
            })
            .collect();
        let evidence = run.results.into_values().collect::<Vec<_>>();
        let baseline_findings =
            baseline::cannot_fail_findings(self, &tree, contract, &digest, &refs, Some(&evidence))
                .await;
        Ok((evidence, Some(commit), missing, baseline_findings))
    }
}
