//! A remediation plateau pauses the observing generation, with its records.
use super::*;

impl WorkflowScriptHost {
    pub(super) fn pause_on_remediation_stall(
        &self,
        record: &WorkflowV2CallRecord,
        view: &str,
        generation: u64,
    ) -> archon_workflow::WorkflowResult<()> {
        if record.call.method != WorkflowV2HostMethod::Checkpoint {
            return Ok(());
        }
        let viewed: serde_json::Value = serde_json::from_str(view)?;
        let Some(evidence) = record
            .call
            .options
            .extra
            .get("remediationPause")
            .or_else(|| viewed.get("remediation_pause"))
        else {
            return Ok(());
        };
        let ids = evidence
            .get("failing_ids")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid("remediation pause needs failing_ids".into())
            })?;
        let mut failing_ids = std::collections::BTreeSet::new();
        for id in ids {
            let id = id
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(
                        "remediation pause ids must be nonempty strings".into(),
                    )
                })?;
            failing_ids.insert(id.to_string());
        }
        if failing_ids.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "a remediation pause needs an unresolved state".into(),
            ));
        }
        let detail = serde_json::json!({
            "event": "remediation_stall_pause", "cause": "no_progress",
            "call_id": record.call.id, "failing_ids": failing_ids,
            "record_path": self.runner.v2_store.result_path(&record.call.id),
            "evidence": evidence,
        });
        let event = archon_workflow::control_pause::pause_with_evidence(
            &self.runner.workflow_store,
            &self.runner.run_id,
            generation,
            detail,
        )?;
        if let Err(error) = event {
            eprintln!("remediation paused; recording its pause event failed: {error}");
        }
        Err(WorkflowError::ControlPaused(format!(
            "remediation made no progress on {}; run {} is paused with its evidence at {}; fix what it names, then workflow resume {}",
            failing_ids.into_iter().collect::<Vec<_>>().join(", "),
            self.runner.run_id,
            self.runner.v2_store.result_path(&record.call.id).display(),
            self.runner.run_id,
        )))
    }
}
