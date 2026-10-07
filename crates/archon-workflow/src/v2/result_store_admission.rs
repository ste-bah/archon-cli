// Durable admission identity belongs to an execution, not its last record.
impl WorkflowV2ResultStore {
    /// Normalize a completion/interruption from its existing admission before
    /// projecting it. The store applies the same rule to every record write.
    pub fn record_with_admission(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> WorkflowResult<WorkflowV2CallRecord> {
        let mut record = record.clone();
        let prior = self.load_call_record(&record.call.id)?.filter(|prior| {
            prior.attempt == record.attempt && prior.input_hash == record.input_hash
        });
        let prior = match prior {
            Some(prior) => Some(prior),
            None => {
                let path = self.admission_path(&record);
                match fs::read(&path) {
                    Ok(bytes) => Some(serde_json::from_slice::<WorkflowV2CallRecord>(&bytes)?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(WorkflowError::io(&path, error)),
                }
            }
        };
        if let Some(prior) = prior {
            if prior.run_id != record.run_id
                || prior.call.id != record.call.id
                || prior.attempt != record.attempt
                || prior.input_hash != record.input_hash
            {
                return Err(WorkflowError::StateCorrupt(
                    "call admission identity mismatch".into(),
                ));
            }
            record.started_at = prior.started_at;
            record.admission_sequence = prior.admission_sequence;
        }
        Ok(record)
    }
    fn admission_path(&self, record: &WorkflowV2CallRecord) -> PathBuf {
        self.admission_path_for(&record.call.id, record.attempt, &record.input_hash)
    }
    fn admission_path_for(&self, call_id: &str, attempt: u32, input_hash: &str) -> PathBuf {
        let identity = format!("{call_id}:{attempt}:{input_hash}");
        self.root
            .join("admissions")
            .join(format!("{}.json", blake3::hash(identity.as_bytes())))
    }
    /// The attempt a NEW dispatch of `call_id` with `input_hash` takes. An
    /// admission that never got a call record (a dispatch an edit superseded,
    /// or one a crash interrupted) used its attempt: the new dispatch takes
    /// the next one, so it gets its own admission order and start time
    /// instead of inheriting the old dispatch's. The old admission stays as
    /// evidence.
    pub fn next_dispatch_attempt(&self, call_id: &str, input_hash: &str) -> WorkflowResult<u32> {
        let mut attempt = self.next_attempt(call_id)?;
        loop {
            let path = self.admission_path_for(call_id, attempt, input_hash);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    attempt = attempt.checked_add(1).ok_or_else(|| {
                        WorkflowError::StateCorrupt(format!(
                            "call {call_id} has no attempt number left"
                        ))
                    })?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(attempt),
                Err(error) => return Err(WorkflowError::io(&path, error)),
            }
        }
    }
    /// Record the start without exposing a pending call as a completed result.
    pub fn save_call_admission(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<()> {
        self.with_session_write_lock(|| {
            let mut clean = record.clone();
            self.preserve_admission(&mut clean)?;
            write_json_synced(&self.admission_path(&clean), &clean)
        })
    }
    fn preserve_admission(&self, record: &mut WorkflowV2CallRecord) -> WorkflowResult<()> {
        *record = self.record_with_admission(record)?;
        if record.status == WorkflowV2Status::Running && record.admission_sequence.is_none() {
            let path = self.root.join("admission-sequence.json");
            let previous: u64 = match fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => return Err(WorkflowError::io(&path, error)),
            };
            let next = previous.checked_add(1).ok_or_else(|| {
                WorkflowError::StateCorrupt("call admission sequence exhausted".into())
            })?;
            // Counter before the admission: a crash may leave a gap, never
            // reuse an identity. Both writes are under the run lock.
            write_json_synced(&path, &next)?;
            record.admission_sequence = Some(next);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "result_store_admission_tests.rs"]
mod admission_tests;
