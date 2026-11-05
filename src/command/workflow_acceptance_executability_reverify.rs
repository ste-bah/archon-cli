//! Frozen-check observation using the probe's typed site failures.
use super::*;
impl HostProbe {
    fn reverify_baseline(&self) -> Option<Baseline> {
        let tasks = self.task_root.as_deref()?;
        let record = archon_workflow::repository_record::read_repository_record(tasks)
            .ok()
            .flatten()?;
        let base = Baseline::for_task_set(&self.repository, tasks)?;
        (base.commit == record.base_commit || Baseline::recorded(&self.repository, tasks).is_some())
            .then_some(base)
    }

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
        self.unproven
            .lock()
            .expect("unproven lock")
            .retain(|id, _| !ids.contains(id));
        // Without the repository record even the current repository selection
        // may be a legacy fallback. Report the permanent missing input before
        // a missing HEAD can hide it behind a generic copy failure.
        if self.task_root.as_deref().is_none_or(|tasks| {
            matches!(
                archon_workflow::repository_record::read_repository_record(tasks),
                Ok(None)
            )
        }) {
            let reason = "repository.lock repository record is missing; restore the decomposition record before reverify".to_string();
            for id in ids {
                self.unproven(id, reason.clone());
            }
            return Ok((
                Vec::new(),
                None,
                ids.iter().map(|id| (id.clone(), reason.clone())).collect(),
                BTreeMap::new(),
            ));
        }
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
            origin: crate::command::workflow_task_set::executability::BaselineOrigin::ProbedTree,
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
        let mut missing: BTreeMap<String, String> = refs
            .iter()
            .filter_map(|reference| {
                run.unrun(&reference.acceptance_id)
                    .map(|why| (reference.acceptance_id.clone(), why))
            })
            .collect();
        let evidence = run.results.into_values().collect::<Vec<_>>();
        let baseline_findings = match self.reverify_baseline() {
            Some(base) => {
                let known = (base.commit == commit).then_some(evidence.as_slice());
                baseline::cannot_fail_findings(self, &base, contract, &digest, &refs, known).await
            }
            None => {
                for reference in &refs {
                    self.unproven(&reference.acceptance_id,
                        "repository.lock pre-implementation record is missing or its base commit is unusable; can-fail proof is unproven".into());
                }
                BTreeMap::new()
            }
        };
        missing.extend(
            self.unproven
                .lock()
                .expect("unproven lock")
                .iter()
                .filter(|(id, _)| ids.contains(*id))
                .map(|(id, (why, _))| (id.clone(), why.clone())),
        );
        Ok((evidence, Some(commit), missing, baseline_findings))
    }
}
