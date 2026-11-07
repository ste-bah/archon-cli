//! Rejected landings remain the author's responsibility in this session.
use super::*;

impl RecordLanding {
    pub fn land(&self, value: Value) -> WorkflowResult<()> {
        self.ensure_writable()?;
        let _lock = self.lock.lock().map_err(invalid)?;
        let subject = value
            .get("subject")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned);
        let removes = value.get("remove").and_then(Value::as_bool) == Some(true);
        // Only a subject this call can land can owe a corrected record.
        // Skeleton authors establish their ids, but must name the same id in task.
        // task is also accepted as a JSON-encoded object by prepare_payload.
        let task_id = value.get("task").and_then(|task| match task {
            Value::String(text) => serde_json::from_str::<Value>(text)
                .ok()?
                .get("task_id")
                .cloned(),
            _ => task.get("task_id").cloned(),
        });
        let trackable = subject.as_ref().is_some_and(|subject| {
            if self.kind == RecordKind::Skeleton {
                task_id.as_ref().and_then(Value::as_str) == Some(subject.as_str())
            } else {
                self.subjects.contains(subject)
            }
        });
        use sha2::{Digest, Sha256};
        let path = subject.as_ref().map(|subject| {
            self.root.join(format!(
                "record-{:x}.json",
                Sha256::digest(subject.as_bytes())
            ))
        });
        let task_bytes = |path: &std::path::Path| -> WorkflowResult<Option<Vec<u8>>> {
            match std::fs::read(path) {
                Ok(bytes) => {
                    let record: StageRecord = serde_json::from_slice(&bytes)?;
                    let bytes = if self.kind == RecordKind::Skeleton {
                        serde_json::to_vec(&record.task)?
                    } else {
                        serde_json::to_vec(&record)?
                    };
                    Ok(Some(Sha256::digest(bytes).to_vec()))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(invalid(error)),
            }
        };
        if let (Some(subject), Some(path)) = (&subject, &path) {
            let mut initial = self.pre_call_tasks.lock().map_err(invalid)?;
            if !initial.contains_key(subject) {
                initial.insert(subject.clone(), task_bytes(path)?);
            }
        }
        let read_record = || -> WorkflowResult<Option<Vec<u8>>> {
            match path.as_ref().map(std::fs::read).transpose() {
                Ok(bytes) => Ok(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(invalid(error)),
            }
        };
        let before_record = if self.kind != RecordKind::Skeleton {
            read_record()?
        } else {
            None
        };
        // Commit the pre-operation image first, so a crash during mutation rolls back.
        self.save_call()?;
        let result = self.land_record(value);
        if let Some(subject) = subject {
            let mut rejected = self.rejected.lock().map_err(invalid)?;
            match &result {
                Ok(()) => {
                    rejected.remove(&subject);
                    if self.kind == RecordKind::Skeleton {
                        let after = task_bytes(path.as_ref().unwrap())?;
                        let initial = self.pre_call_tasks.lock().map_err(invalid)?;
                        let mut landed = self.landed_here.lock().map_err(invalid)?;
                        if initial.get(&subject) != Some(&after) {
                            landed.insert(subject);
                        } else {
                            landed.remove(&subject);
                        }
                    } else if before_record != read_record()? {
                        self.landed_here.lock().map_err(invalid)?.insert(subject);
                    }
                }
                Err(error) if !removes && trackable => {
                    rejected.insert(subject, vec![error.to_string()]);
                }
                Err(_) => {}
            }
        }
        self.save_call()?;
        result
    }

    pub(crate) fn require_repaired(&self) -> WorkflowResult<()> {
        let rejected = self.rejected.lock().map_err(invalid)?;
        if rejected.is_empty() {
            return Ok(());
        }
        let errors = rejected
            .iter()
            .map(|(subject, errors)| format!("{subject}: {}", errors.join("\n")))
            .collect::<Vec<_>>()
            .join("\n");
        Err(invalid(format!(
            "rejected record subjects must be re-landed before completion:\n{errors}"
        )))
    }
}
