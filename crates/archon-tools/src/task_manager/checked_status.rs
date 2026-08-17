pub use super::TaskTransitionError;
use super::{TaskManager, TaskStatus, plan_persistence};

impl TaskManager {
    /// Complete a plan-linked task using durable evidence identities supplied in
    /// [`RequiredEvidence`]. Its kind/status/sequence fields are ignored; the
    /// durable record and its verifier provenance are resolved independently.
    pub fn set_status_checked(
        &self,
        id: &str,
        status: TaskStatus,
        evidence: &[archon_completion::RequiredEvidence],
    ) -> Result<(), TaskTransitionError> {
        let task = self
            .get_task(id)
            .ok_or_else(|| TaskTransitionError::NotFound(id.to_string()))?;
        let trusted = match &task.metadata {
            Some(metadata) if status == TaskStatus::Completed => {
                let (run_id, evidence_ids) = evidence.iter().try_fold(
                    (None::<String>, Vec::with_capacity(evidence.len())),
                    |(run_id, mut evidence_ids), supplied| {
                        let supplied_run_id = supplied.run_id.as_deref().ok_or_else(|| {
                            TaskTransitionError::UntrustedEvidence(
                                "completion evidence is missing its durable run ID".to_string(),
                            )
                        })?;
                        let evidence_id = supplied.evidence_id.as_ref().ok_or_else(|| {
                            TaskTransitionError::UntrustedEvidence(
                                "completion evidence is missing its durable ID".to_string(),
                            )
                        })?;
                        if run_id
                            .as_deref()
                            .is_some_and(|existing| existing != supplied_run_id)
                        {
                            return Err(TaskTransitionError::UntrustedEvidence(
                                "completion evidence spans multiple durable runs".to_string(),
                            ));
                        }
                        evidence_ids.push(evidence_id.clone());
                        Ok((Some(supplied_run_id.to_string()), evidence_ids))
                    },
                )?;
                let (store, authority) = {
                    let persistence = self
                        .plan_persistence
                        .lock()
                        .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
                    let store =
                        persistence
                            .get(&metadata.session_id)
                            .cloned()
                            .ok_or_else(|| {
                                TaskTransitionError::Persistence(format!(
                                    "plan-linked task session {} has no attached plan store",
                                    metadata.session_id
                                ))
                            })?;
                    drop(persistence);
                    let authorities = self
                        .plan_authorities
                        .lock()
                        .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
                    let authority = authorities.get(&metadata.session_id).cloned().ok_or_else(
                        || {
                            TaskTransitionError::Persistence(format!(
                                "plan-linked task session {} has no attached approval authority",
                                metadata.session_id
                            ))
                        },
                    )?;
                    (store, authority)
                };
                store
                    .resolve_required_evidence(
                        &authority,
                        &metadata.session_id,
                        run_id.as_deref().unwrap_or_default(),
                        &evidence_ids,
                        &metadata.required_evidence,
                    )
                    .map_err(|error| {
                        if error.kind() == std::io::ErrorKind::PermissionDenied {
                            TaskTransitionError::UntrustedEvidence(error.to_string())
                        } else {
                            TaskTransitionError::EvidenceResolution(error.to_string())
                        }
                    })?
            }
            _ => Vec::new(),
        };
        plan_persistence::set_status_checked(self, id, status, &trusted)
    }
}
