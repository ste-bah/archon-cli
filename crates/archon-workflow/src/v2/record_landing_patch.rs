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
        let mut identities = BTreeMap::new();
        for task in skeleton.tasks {
            let record = json!({"subject":task.task_id,"task":task});
            let parsed: StageRecord = serde_json::from_value(record.clone())?;
            if let Err(error) = self.validate(&parsed) {
                tracing::warn!(subject = %parsed.subject, %error, "ignored invalid skeleton seed task");
                continue;
            }
            identities.insert(task.task_id.clone(), task.file_name.clone());
            let path = self.record_path(&task.task_id);
            // Existing landed tasks supersede the seed.
            if !path.exists() {
                write(&path, &record)?;
            }
        }
        if frozen {
            *self.frozen_tasks.lock().map_err(invalid)? = Some(identities);
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
