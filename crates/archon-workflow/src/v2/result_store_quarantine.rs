// A call's own result slot, read so that damage is never mistaken for absence
// (Issue 313).
//
// The directory scans skip a file that is not shaped like one of this store's
// records (`result_store_scan.rs`): there a foreign note must not fail the
// walk. A caller that asks for ONE call's slot is different. A file at that
// exact path is the call's record or the damaged remains of it, and reading it
// as "no record" says the call never ran. So here every file that does not
// read back as the call's own record is damage, whatever its bytes are: not
// JSON, JSON of the wrong shape, or the record of another call.
//
// Damage is QUARANTINED, never deleted: the bytes move to
// `results/quarantine/<slot stem>/` beside an evidence file naming the slot
// and why, evidence first (a crash between the two leaves the record in place,
// to be quarantined again). The emptied slot then reads as damaged, not empty,
// while that evidence stands, so a later read in the same finalization knows
// the call ran. A new execution of the call takes the slot back.

/// The evidence event's name, in its file and in the run's events.
pub const CALL_QUARANTINE_EVENT: &str = "call_record_quarantined";

const CALL_QUARANTINE: &str = "quarantine";
const CALL_EVIDENCE_SUFFIX: &str = ".evidence.json";
const CALL_DAMAGED_SUFFIX: &str = ".damaged";

/// What was quarantined from a call's slot, and why: the evidence file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantinedCallRecordV1 {
    pub event: String,
    pub call_id: String,
    /// Where the record was, relative to the store root.
    pub original: String,
    /// Where its bytes are now, relative to the store root.
    pub quarantined: String,
    pub reason: String,
    pub quarantined_at: String,
}

/// A call's slot as [`WorkflowV2ResultStore::load_call_slot_healing`] found it.
#[derive(Debug, Clone)]
pub enum WorkflowV2CallSlot {
    /// The call's own record.
    Whole(Box<WorkflowV2CallRecord>),
    /// No record, and none quarantined: the call recorded nothing.
    Empty,
    /// The slot held damage, now quarantined: by this read (`fresh`) or by
    /// an earlier one.
    Damaged {
        evidence: QuarantinedCallRecordV1,
        fresh: bool,
    },
}

impl WorkflowV2ResultStore {
    /// Where `call_id`'s damaged slot records go.
    pub fn call_quarantine_dir(&self, call_id: &str) -> PathBuf {
        let slot = self.result_path(call_id);
        let stem = slot
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("record");
        self.root.join("results").join(CALL_QUARANTINE).join(stem)
    }

    /// `call_id`'s slot. A file there that is not the call's own record is
    /// quarantined and reported as damaged; a slot emptied by an earlier
    /// quarantine is damaged too. An I/O error is returned, and nothing moves.
    pub fn load_call_slot_healing(&self, call_id: &str) -> WorkflowResult<WorkflowV2CallSlot> {
        let path = self.result_path(call_id);
        let raw = match fs::read(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return self.quarantined_slot(call_id);
            }
            Err(err) => return Err(WorkflowError::io(&path, err)),
        };
        let reason = match serde_json::from_slice::<WorkflowV2CallRecord>(&raw) {
            Ok(record) if record.call.id == call_id => {
                return Ok(WorkflowV2CallSlot::Whole(Box::new(record)));
            }
            Ok(record) => format!("it holds the record of call {}", record.call.id),
            Err(error) => format!("it does not read back as a call record: {error}"),
        };
        match self.quarantine_call_slot(call_id, &path, reason)? {
            Some(evidence) => Ok(WorkflowV2CallSlot::Damaged {
                evidence,
                fresh: true,
            }),
            // Another reader moved it first.
            None => self.quarantined_slot(call_id),
        }
    }

    /// The empty slot of `call_id`: damaged while the evidence of its
    /// newest quarantine stands beside the moved bytes.
    fn quarantined_slot(&self, call_id: &str) -> WorkflowResult<WorkflowV2CallSlot> {
        let dir = self.call_quarantine_dir(call_id);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(WorkflowV2CallSlot::Empty);
            }
            Err(err) => return Err(WorkflowError::io(&dir, err)),
        };
        let mut newest: Option<QuarantinedCallRecordV1> = None;
        for entry in entries {
            let path = entry.map_err(|err| WorkflowError::io(&dir, err))?.path();
            let is_evidence = (path.file_name().and_then(|name| name.to_str()))
                .is_some_and(|name| name.ends_with(CALL_EVIDENCE_SUFFIX));
            if !is_evidence {
                continue;
            }
            let raw = fs::read(&path).map_err(|err| WorkflowError::io(&path, err))?;
            let Ok(evidence) = serde_json::from_slice::<QuarantinedCallRecordV1>(&raw) else {
                tracing::warn!(path = %path.display(), "call quarantine evidence will not parse");
                continue;
            };
            let moved =
                evidence.call_id == call_id && self.root.join(&evidence.quarantined).is_file();
            if moved
                && newest
                    .as_ref()
                    .is_none_or(|n| n.quarantined_at < evidence.quarantined_at)
            {
                newest = Some(evidence);
            }
        }
        Ok(newest.map_or(WorkflowV2CallSlot::Empty, |evidence| {
            WorkflowV2CallSlot::Damaged {
                evidence,
                fresh: false,
            }
        }))
    }

    /// Moves the damaged slot `path` into the call's quarantine, evidence
    /// first; `None` when another reader moved it first.
    fn quarantine_call_slot(
        &self,
        call_id: &str,
        path: &Path,
        reason: String,
    ) -> WorkflowResult<Option<QuarantinedCallRecordV1>> {
        let dir = self.call_quarantine_dir(call_id);
        fs::create_dir_all(&dir).map_err(|err| WorkflowError::io(&dir, err))?;
        let stem = uuid::Uuid::new_v4().to_string();
        let moved = dir.join(format!("{stem}{CALL_DAMAGED_SUFFIX}"));
        let relative = |path: &Path| {
            let path = path.strip_prefix(&self.root).unwrap_or(path);
            path.to_string_lossy().replace('\\', "/")
        };
        let evidence = QuarantinedCallRecordV1 {
            event: CALL_QUARANTINE_EVENT.to_string(),
            call_id: call_id.to_string(),
            original: relative(path),
            quarantined: relative(&moved),
            reason,
            quarantined_at: chrono::Utc::now().to_rfc3339(),
        };
        crate::store::write_atomic(
            &dir.join(format!(".{stem}.tmp")),
            &dir.join(format!("{stem}{CALL_EVIDENCE_SUFFIX}")),
            &serde_json::to_vec_pretty(&evidence)?,
        )?;
        match crate::store::rename_durable(path, &moved) {
            Err(WorkflowError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            other => other?,
        }
        crate::store::sync_dir(&dir)?;
        if let Some(results) = path.parent() {
            crate::store::sync_dir(results)?;
        }
        tracing::warn!(
            call_id,
            record = %evidence.original,
            moved_to = %evidence.quarantined,
            reason = %evidence.reason,
            "damaged call record quarantined"
        );
        Ok(Some(evidence))
    }
}
