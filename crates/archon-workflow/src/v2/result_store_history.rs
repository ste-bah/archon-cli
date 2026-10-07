// The last accepted record of a call, read from the call's whole history
// (Issue-250).
//
// A call has one result slot (`result_path`). A new attempt takes the slot
// when it starts (the `running` record of a fixed run) and keeps it when a
// pause, a cancel or a dead host stops it. The accepted record it displaced
// then lives on only in the call's archive (D79), its own directory under
// `results/history/` (Issue-254). Reuse read the slot alone, so an
// interrupted re-run lost accepted work: the next resume, asked the very
// input that record answered, found no answer and ran the call again.
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

/// The archived records of one call (its [`WorkflowV2ResultStore::call_history_dir`]).
struct ArchivedCallRecords {
    records: Vec<(PathBuf, WorkflowV2CallRecord)>,
    /// Archived files of the call that do not read back as records, with the
    /// attempt number when their JSON still names one.
    unreadable: Vec<(PathBuf, Option<u32>)>,
}

/// `raw` as one of this store's call records, or `None` when it is not one.
fn parse_call_record(raw: &str, path: &Path) -> Option<WorkflowV2CallRecord> {
    parse_store_record::<WorkflowV2CallRecord>(raw, path)
        .ok()
        .flatten()
}

impl WorkflowV2ResultStore {
    /// Every archived record of `call_id`.
    ///
    /// Cost (Issue-254): one listing of the call's own archive directory
    /// ([`Self::call_history_dir`]) and a read of each file in it; no other
    /// call's archive is listed.
    fn archived_call_records(&self, call_id: &str) -> WorkflowResult<ArchivedCallRecords> {
        self.migrate_flat_archive()?;
        let mut archived = ArchivedCallRecords {
            records: Vec::new(),
            unreadable: Vec::new(),
        };
        let dir = self.call_history_dir(call_id);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(archived),
            Err(err) => return Err(WorkflowError::io(&dir, err)),
        };
        for entry in entries {
            let path = entry.map_err(|err| WorkflowError::io(&dir, err))?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            // An entry `store_file` refuses is reported and, like any file
            // that will not read, leaves a gap in the call's history.
            let raw =
                read_store_file_or_report(&path, &dir).and_then(|raw| String::from_utf8(raw).ok());
            match raw.as_deref().and_then(|raw| parse_call_record(raw, &path)) {
                Some(record) if record.call.id == call_id => archived.records.push((path, record)),
                Some(_) => {}
                None => {
                    let attempt = raw
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                        .and_then(|value| value.get("attempt")?.as_u64())
                        .and_then(|attempt| u32::try_from(attempt).ok());
                    archived.unreadable.push((path, attempt));
                }
            }
        }
        Ok(archived)
    }

    /// Every record of `call_id`: the slot's and each archived one. `None`
    /// when an archived file of the call cannot be read, or when the slot
    /// holds an entry the store's readers refuse (Issue-292: a directory, a
    /// FIFO, a link out of `results/`): a history with a gap proves nothing,
    /// so an older archived record never answers in its place.
    fn call_history(&self, call_id: &str) -> WorkflowResult<Option<Vec<WorkflowV2CallRecord>>> {
        let archived = self.archived_call_records(call_id)?;
        if !archived.unreadable.is_empty() || self.slot_refused(call_id) {
            return Ok(None);
        }
        let mut records = self
            .load_call_record(call_id)?
            .into_iter()
            .collect::<Vec<_>>();
        records.extend(archived.records.into_iter().map(|(_, record)| record));
        Ok(Some(records))
    }

    /// Whether `call_id`'s slot holds something that is not a readable
    /// record file: a refused entry, or a link that does not resolve.
    fn slot_refused(&self, call_id: &str) -> bool {
        let slot = self.result_path(call_id);
        let root = slot.parent().unwrap_or(&self.root);
        fs::symlink_metadata(&slot).is_ok()
            && !matches!(
                super::store_file::classify_store_entry(&slot, root),
                Ok(None)
            )
    }

    /// Every call record this store holds: each slot's and every readable
    /// archived one. The restart selection reads this, so a call whose slot
    /// an interrupted attempt took is still found by the tasks its earlier
    /// records did. An unreadable archived file is skipped here; its call
    /// has a gap in its history and is never answered from it.
    pub(super) fn load_call_record_history(&self) -> WorkflowResult<Vec<WorkflowV2CallRecord>> {
        let mut records = self.load_call_records()?;
        self.migrate_flat_archive()?;
        let root = self.call_archive_root();
        let calls = match fs::read_dir(&root) {
            Ok(calls) => calls,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(records),
            Err(err) => return Err(WorkflowError::io(&root, err)),
        };
        for call_dir in calls {
            let dir = call_dir
                .map_err(|err| WorkflowError::io(&root, err))?
                .path();
            // What cannot be listed or read is reported, never skipped
            // silently; its call has a gap (above).
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) => {
                    report_skipped_store_entry(&dir, &err);
                    continue;
                }
            };
            for entry in entries {
                let path = match entry {
                    Ok(entry) => entry.path(),
                    Err(err) => {
                        report_skipped_store_entry(&dir, &err);
                        continue;
                    }
                };
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                if let Some(record) = read_store_file_or_report(&path, &dir)
                    .and_then(|raw| String::from_utf8(raw).ok())
                    .and_then(|raw| parse_call_record(&raw, &path))
                {
                    records.push(record);
                }
            }
        }
        Ok(records)
    }

    /// Mark `call_id` invalidated by `reason` wherever a record of it could
    /// answer a resume: its slot, and every accepted-grade archived record
    /// not already invalidated (rewritten in place, temporary file and
    /// rename). An explicit restart is thus never undone by history reuse,
    /// whatever holds the slot -- an interrupted attempt, or nothing after a
    /// crash. `true` when a record was marked.
    pub(super) fn invalidate_call_everywhere(
        &self,
        call_id: &str,
        reason: &str,
    ) -> WorkflowResult<bool> {
        let mut marked = false;
        if let Some(mut record) = self.load_call_record(call_id)? {
            record.invalidated_by = Some(reason.to_string());
            self.save_call_record(&record)?;
            marked = true;
        }
        for (path, mut record) in self.archived_call_records(call_id)?.records {
            if record.invalidated_by.is_none() && accepted_grade(&record) {
                record.invalidated_by = Some(reason.to_string());
                self.write_record(&path, &record)?;
                marked = true;
            }
        }
        Ok(marked)
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
        archive_superseded_json_into(
            &path,
            &self.call_history_dir(&record.call.id),
            self.durable,
            |existing: &WorkflowV2CallRecord| existing == record,
        )?;
        self.write_record(&path, record)
    }

    /// The attempt number the next execution of `call_id` takes: one past
    /// every attempt on record, archived ones included, so a record restored
    /// into the slot never gives its old number to a new attempt. An
    /// archived file that no longer reads as a record still counts with the
    /// attempt its JSON names; one that names none is reported and skipped.
    pub fn next_attempt(&self, call_id: &str) -> WorkflowResult<u32> {
        let archived = self.archived_call_records(call_id)?;
        let mut latest = self.load_call_record(call_id)?.map(|record| record.attempt);
        for attempt in archived.records.iter().map(|(_, record)| record.attempt) {
            latest = latest.max(Some(attempt));
        }
        for (path, attempt) in &archived.unreadable {
            match attempt {
                Some(attempt) => latest = latest.max(Some(*attempt)),
                None => eprintln!(
                    "workflow v2 call '{call_id}': archived record {} names no attempt; the next attempt number ignores it",
                    path.display()
                ),
            }
        }
        Ok(latest.map_or(1, |attempt| attempt.saturating_add(1)))
    }
}
