use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReauthorResult {
    pub(crate) acceptance_digest: String,
    pub(crate) freeze_event_id: String,
    pub(crate) skeleton_digest: Option<String>,
    pub(crate) diagnostics: Vec<String>,
    /// Checks still pending, with their latest host or authored finding.
    pub(crate) unproven: BTreeMap<String, String>,
    pub(crate) host_faults: BTreeMap<String, crate::command::workflow_host_fault::Class>,
    /// The subset pending because their authored check or judge reply needs
    /// another attempt. These findings do not represent host faults.
    pub(crate) authored_pending: BTreeMap<String, String>,
    pub(crate) published: bool,
}

/// Publication committed; only settlement, archival and readback may run again.
#[derive(Debug)]
pub(crate) struct CommittedRepair {
    pub(crate) result: ReauthorResult,
}
impl std::fmt::Display for CommittedRepair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "repair {} committed; finish settlement and archival",
            self.result.acceptance_digest
        )
    }
}
impl std::error::Error for CommittedRepair {}
impl CommittedRepair {
    pub(crate) fn settle(
        &self,
        project: &std::path::Path,
        tasks: &std::path::Path,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        let _read = super::super::ChainRead::of(project, tasks)?;
        let path = super::super::acceptance_pin_path(project, tasks);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading committed repair pin {}", path.display()))?;
        let pin: archon_workflow::task_set_contract::AcceptancePin = serde_json::from_slice(&bytes)
            .with_context(|| format!("restore valid committed repair pin {}", path.display()))?;
        if pin.acceptance_digest != self.result.acceptance_digest
            || pin.freeze_event_id != self.result.freeze_event_id
            || pin.skeleton_digest != self.result.skeleton_digest
        {
            anyhow::bail!(
                "committed repair pin {} changed; restore the recorded committed repair {}",
                path.display(),
                self.result.acceptance_digest
            );
        }
        if self.result.unproven.is_empty() {
            super::super::reauthor::cache::archive(tasks, &self.result.acceptance_digest)
                .with_context(|| {
                    format!("archiving consumed candidates under {}", tasks.display())
                })?;
        }
        Ok(())
    }
}
