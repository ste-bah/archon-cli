// The last accepted record of a call, read from the call's whole history
// (Issue-250).
//
// A call has one result slot (`result_path`). A new attempt takes the slot
// when it starts (the `running` record of a fixed run) and keeps it when a
// pause, a cancel or a dead host stops it. The accepted record it displaced
// then lives on only in `results/superseded/` (D79). Reuse read the slot
// alone, so an interrupted re-run lost accepted work: the next resume, asked
// the very input that record answered, found no answer and ran the call again.
//
// The history here is the slot plus every archived record of the call. Its
// last accepted record answers a resume while nothing after it took its
// place: no later finished answer to the same input, and no invalidation.

/// A stored record for the reuse checks to judge for one call.
#[derive(Debug, Clone)]
pub struct WorkflowV2ReuseCandidate {
    pub record: WorkflowV2CallRecord,
    /// The record came from the call's history, not from its slot. Once it is
    /// reused it goes back into the slot ([`WorkflowV2ResultStore::restore_call_record`]).
    pub from_history: bool,
}

/// An attempt that ended without an answer: still `running` or `pending`
/// (the record a fixed run writes at dispatch), or stopped by a pause, a
/// cancel or a dead host (`result.data.interrupted` names the reason).
pub fn is_unfinished_attempt(record: &WorkflowV2CallRecord) -> bool {
    matches!(
        record.status,
        WorkflowV2Status::Running | WorkflowV2Status::Pending
    ) || record
        .result
        .data
        .get("interrupted")
        .is_some_and(serde_json::Value::is_string)
}

/// A final answer as good as an accepted one: accepted, no-op, or a review
/// map whose every branch finished its review.
fn accepted_grade(record: &WorkflowV2CallRecord) -> bool {
    crate::v2::script::is_reusable_status(record.status)
        || crate::v2::script::completed_review_map_record(record)
}

/// When `record` finished, in nanoseconds since the epoch.
fn finished_nanos(record: &WorkflowV2CallRecord) -> Option<i64> {
    let at = if record.finished_at.is_empty() {
        &record.started_at
    } else {
        &record.finished_at
    };
    chrono::DateTime::parse_from_rfc3339(at)
        .ok()?
        .timestamp_nanos_opt()
}

impl WorkflowV2ResultStore {
    /// Every record of `call_id`: the slot's and each archived one. `None`
    /// when an archived record of the call cannot be read: a history with a
    /// gap proves nothing.
    fn call_history(&self, call_id: &str) -> WorkflowResult<Option<Vec<WorkflowV2CallRecord>>> {
        let slot = self.result_path(call_id);
        let mut records = Vec::new();
        if slot.exists()
            && let Some(record) = read_store_record::<WorkflowV2CallRecord>(&slot)?
        {
            records.push(record);
        }
        let dir = self.root.join("results").join("superseded");
        // `archive_superseded_json` names an archived record after the slot's
        // stem, a dash, and a unique suffix.
        let prefix = format!(
            "{}-",
            slot.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
        );
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Some(records)),
            Err(err) => return Err(WorkflowError::io(&dir, err)),
        };
        for entry in entries {
            let entry = entry.map_err(|err| WorkflowError::io(&dir, err))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with(&prefix) || !name.ends_with(".json") {
                continue;
            }
            let path = entry.path();
            let parsed = fs::read_to_string(&path).ok().and_then(|raw| {
                parse_store_record::<WorkflowV2CallRecord>(&raw, &path)
                    .ok()
                    .flatten()
            });
            match parsed {
                Some(record) if record.call.id == call_id => records.push(record),
                Some(_) => {}
                None => return Ok(None),
            }
        }
        Ok(Some(records))
    }

    /// The last accepted record of `call_id`, when it answers `input_hash`
    /// and nothing recorded after it took its place.
    ///
    /// The last accepted record is the latest-finished accepted-grade record
    /// of any input: a newer accepted attempt replaces an older one. It
    /// answers only when it is reusable for `input_hash`, when no record of
    /// the call was invalidated at or after it finished (an invalidation is
    /// never undone), and when no finished record with the same input came at
    /// or after it (a later verdict on the same input wins). Unfinished
    /// attempts ([`is_unfinished_attempt`]) answer nothing and displace
    /// nothing. A record without a readable finish time leaves the order
    /// unknown, so there is no answer.
    pub fn last_accepted_call_record(
        &self,
        call_id: &str,
        input_hash: &str,
    ) -> WorkflowResult<Option<WorkflowV2CallRecord>> {
        let Some(history) = self.call_history(call_id)? else {
            return Ok(None);
        };
        let mut timed = Vec::with_capacity(history.len());
        for record in history {
            let Some(at) = finished_nanos(&record) else {
                return Ok(None);
            };
            timed.push((at, record));
        }
        let Some((at, last)) = timed
            .iter()
            .filter(|(_, record)| accepted_grade(record))
            .max_by_key(|(at, _)| *at)
        else {
            return Ok(None);
        };
        let displaced = timed.iter().any(|(other_at, other)| {
            let invalidated = other.invalidated_by.is_some();
            let answered_again = other.input_hash == last.input_hash
                && !accepted_grade(other)
                && !is_unfinished_attempt(other);
            other_at >= at && (invalidated || answered_again)
        });
        Ok((!displaced && last.is_reusable_for(input_hash)).then(|| last.clone()))
    }

    /// The record the reuse checks judge for `call`: the slot's, unless the
    /// slot holds no accepted-grade answer and the call's last accepted
    /// record answers `input_hash` ([`Self::last_accepted_call_record`]).
    ///
    /// Three kinds of call keep the slot alone. Remediation work: its replay
    /// rules pair records by when each file of its label was written, and an
    /// older record put back into the slot would rewrite that order. A call
    /// with branch outcomes on file: a later attempt may have rewritten them,
    /// so they need not belong to the older record (its branches are reused
    /// one by one instead). A host command: its fixed-run replay rules judge
    /// the slot as the latest execution.
    pub fn call_record_for_reuse(
        &self,
        call: &WorkflowV2HostCall,
        input_hash: &str,
    ) -> WorkflowResult<Option<WorkflowV2ReuseCandidate>> {
        let slot = self.load_call_record(&call.id)?;
        let slot_answers = slot.as_ref().is_some_and(accepted_grade);
        let keeps_slot = crate::v2::script::resume_drift::is_remediation_call(call)
            || call.method == super::WorkflowV2HostMethod::HostCommand
            || self
                .root
                .join("branches")
                .join(sanitize_call_id(&call.id))
                .exists();
        if !slot_answers
            && !keeps_slot
            && let Some(record) = self.last_accepted_call_record(&call.id, input_hash)?
        {
            return Ok(Some(WorkflowV2ReuseCandidate {
                record,
                from_history: true,
            }));
        }
        Ok(slot.map(|record| WorkflowV2ReuseCandidate {
            record,
            from_history: false,
        }))
    }

    /// Put `record`, read from its call's history, back into the call's
    /// slot. Whatever holds the slot now is archived first (D79), then the
    /// record is written exactly as it was (temporary file and rename). A
    /// crash between the two leaves an empty slot and both records in the
    /// history, where the next resume finds the accepted one again. This
    /// session's ledger is not touched: the record is an earlier execution,
    /// not a new one.
    pub fn restore_call_record(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<()> {
        let path = self.result_path(&record.call.id);
        archive_superseded_json(&path, |existing: &WorkflowV2CallRecord| existing == record)?;
        write_json(&path, record)
    }

    /// The attempt number the next execution of `call_id` takes: one past
    /// every attempt on record, archived ones included, so a record restored
    /// into the slot never gives its old number to a new attempt.
    pub fn next_attempt(&self, call_id: &str) -> WorkflowResult<u32> {
        let latest = match self.call_history(call_id)? {
            Some(history) => history.iter().map(|record| record.attempt).max(),
            None => self.load_call_record(call_id)?.map(|record| record.attempt),
        };
        Ok(latest.map_or(1, |attempt| attempt.saturating_add(1)))
    }
}
