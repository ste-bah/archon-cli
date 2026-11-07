//! Skeleton re-authoring changes only explicitly re-landed records.
use super::*;

impl RecordLanding {
    pub(crate) fn skeleton_is_frozen(&self) -> WorkflowResult<bool> {
        Ok(self.frozen_tasks.lock().map_err(invalid)?.is_some())
    }

    pub fn seed_skeleton(&self, previous: &Value, frozen: bool) -> WorkflowResult<()> {
        if self.kind != RecordKind::Skeleton {
            return Err(invalid("only skeleton calls may be seeded"));
        }
        let skeleton: crate::task_skeleton::TaskSkeleton =
            match serde_json::from_value(previous.clone()) {
                Ok(skeleton) => skeleton,
                Err(error) => {
                    tracing::warn!(%error, "ignored malformed skeleton seed");
                    return Ok(());
                }
            };
        let _lock = self.lock.lock().map_err(invalid)?;
        self.save_call()?;
        let mut identities = BTreeMap::new();
        for task in skeleton.tasks {
            let record = json!({"subject":task.task_id,"task":task});
            let parsed: StageRecord = serde_json::from_value(record.clone())?;
            if let Err(error) = self.validate_skeleton_record(&parsed, Admission::Persisted) {
                tracing::warn!(subject = %parsed.subject, %error, "ignored invalid skeleton seed task");
                continue;
            }
            identities.insert(task.task_id.clone(), task.file_name.clone());
            let path = self.record_path(&task.task_id);
            // Existing landed tasks supersede the seed.
            if !path.exists()
                && !self
                    .pre_call_tasks
                    .lock()
                    .map_err(invalid)?
                    .contains_key(&task.task_id)
            {
                write(&path, &record)?;
            }
        }
        if frozen {
            *self.frozen_tasks.lock().map_err(invalid)? = Some(identities);
        }
        self.save_call()
    }

    // Frozen seeds retain repairable contracts; only new landings audit strength.
    pub(super) fn validate_skeleton_record(
        &self,
        r: &StageRecord,
        admission: Admission,
    ) -> WorkflowResult<()> {
        if r.subject.trim().is_empty() || r.subject.len() > 4096 {
            return Err(invalid("record subject is missing or too long"));
        }
        let task = r
            .task
            .as_ref()
            .ok_or_else(|| invalid("skeleton record requires task"))?;
        if task.task_id != r.subject || task.file_name.is_empty() {
            return Err(invalid("skeleton task identity mismatch"));
        }
        crate::repository_audit::contract::validate_path(&task.file_name).map_err(invalid)?;
        // Reject tasks that name no obligations or deliverables.
        if task.implements.is_empty() && task.deliverable_contracts.is_empty() {
            return Err(invalid(
                "a skeleton task must name the obligations it implements and/or the artifacts it delivers; a stub with neither cannot be owned or verified",
            ));
        }
        // Name hollow entries before auditing contract strength.
        if let Some(i) = task.implements.iter().position(|id| id.trim().is_empty()) {
            return Err(invalid(format!(
                "task.implements[{i}] is blank; each entry must be an obligation id"
            )));
        }
        for (i, contract) in task.deliverable_contracts.iter().enumerate() {
            if contract.kind.trim().is_empty() {
                return Err(invalid(format!(
                    "task.deliverable_contracts[{i}].kind is blank; each contract must name its kind"
                )));
            }
            if contract.artifact_path.trim().is_empty() {
                return Err(invalid(format!(
                    "task.deliverable_contracts[{i}].artifact_path is blank; each contract must name the artifact it delivers"
                )));
            }
        }
        let skeleton = crate::task_skeleton::TaskSkeleton {
            schema_version: 1,
            acceptance_digest: "pending".into(),
            tasks: vec![task.clone()],
        };
        let defects = if matches!(admission, Admission::NewLanding) {
            crate::task_skeleton::skeleton_defects(&skeleton, "pending")
        } else {
            crate::task_skeleton::skeleton_shape_defects(&skeleton, "pending")
        };
        if !defects.is_empty() {
            return Err(invalid(crate::defect::defect_message(&defects)));
        }

        Ok(())
    }

    fn record_path(&self, subject: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        self.root.join(format!(
            "record-{:x}.json",
            Sha256::digest(subject.as_bytes())
        ))
    }

    pub(super) fn validate_patch(&self, record: &StageRecord) -> WorkflowResult<()> {
        if record.remove && (record.subject.trim().is_empty() || record.subject.len() > 4096) {
            return Err(invalid("record subject is missing or too long"));
        }
        if record.remove && self.kind != RecordKind::Skeleton {
            return Err(invalid("remove is only available for draft skeleton tasks"));
        }
        if let Some(tasks) = self.frozen_tasks.lock().map_err(invalid)?.as_ref() {
            if record.remove {
                return Err(invalid("missing_frozen_task: cannot remove a frozen task"));
            }
            if record
                .task
                .as_ref()
                .is_some_and(|task| task.task_id != record.subject)
            {
                return Err(invalid(
                    "missing_frozen_task: preserve every frozen task_id",
                ));
            }
            if record.task.as_ref().is_some_and(|task| {
                tasks
                    .get(&record.subject)
                    .is_some_and(|file_name| &task.file_name != file_name)
            }) {
                return Err(invalid(
                    "frozen_field_changed: preserve the frozen file_name",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn remove_task(&self, subject: &str) -> WorkflowResult<()> {
        let path = self.record_path(subject);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A rejected first landing may be deliberately withdrawn in a draft.
                if self.rejected.lock().map_err(invalid)?.contains_key(subject) {
                    Ok(())
                } else {
                    Err(invalid(format!(
                        "cannot remove unknown draft task {subject}"
                    )))
                }
            }
            Err(error) => Err(invalid(error)),
        }
    }
}
