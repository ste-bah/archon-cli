//! Issue-550: the one final-answer contract of a call that lands records.
//!
//! A live skeleton author was shown two answer contracts at once — the whole
//! artifact and the landing protocol — and lost three of seven tries to the
//! mix: a count without the digest placeholder, a single record as the
//! answer, and a loop of re-landings. When records are landed the host owns
//! the artifact: it composes it from the retained records, every receipt ends
//! with the exact final reply, and a session that cannot give that reply can
//! still be completed from what it retained.
use super::record_landing::{RecordKind, RecordLanding, StageRecord, invalid, schema_hint};
use super::record_landing_schema::field_list;
use crate::WorkflowResult;
use serde_json::{Value, json};
use std::collections::BTreeMap;

impl RecordLanding {
    /// The receipt for skeleton records: no fixed subjects, no findings, a
    /// re-landing replaces its task, and the last words are the exact reply.
    pub(super) fn skeleton_hint(
        &self,
        records: &BTreeMap<String, StageRecord>,
    ) -> WorkflowResult<String> {
        let new_count = self.new_landing_count()?;
        let pending = self.require_repaired().err();
        let last = if let Some(error) = pending {
            let action = if self.skeleton_is_frozen()? {
                "Re-land valid tasks under these subject ids, preserving frozen task ids and file names, before replying."
            } else {
                "Correct or withdraw these tasks before replying."
            };
            format!("Completion is pending: {error}. {action}")
        } else if let Err(error) = self.require_finding_repairs() {
            format!("Completion is pending: {error}")
        } else if records.is_empty() {
            "Land each task as one record; every landing receipt ends with the exact final reply."
                .to_owned()
        } else if new_count == 0 {
            "When no task is left to land, reply with {\"records_landed\":N}, where N is the new landings count from the last receipt (seeds do not count).".to_owned()
        } else {
            format!(
                "With no task left to land, your final reply is exactly {}",
                json!({"records_landed": new_count})
            )
        };
        Ok(format!(
            "Host retained {} records; {} new landings in this call (seeds do not count). An author call with open findings must make at least one byte-changing landing versus the pre-call snapshot: change a task record. Identical task bytes or restoring original task bytes do not count. Landed subjects: {}. Use {} with {}; each subject is a task id you establish, and landing a subject again replaces its task. Schema: {}. The host composes schema_version, acceptance_digest and tasks from the retained records: never send acceptance_digest, never send a record as your final reply. {last}",
            records.len(),
            new_count,
            serde_json::to_string(&records.keys().collect::<Vec<_>>())?,
            self.tool_name(),
            field_list(self.kind),
            schema_hint(self.kind),
        ))
    }

    /// Open findings require at least one byte-changing landing in this call.
    pub fn require_finding_landings(&self, findings: &[Value]) -> WorkflowResult<()> {
        self.require_new_landings(!findings.is_empty())
    }

    fn require_finding_repairs(&self) -> WorkflowResult<()> {
        if !*self.repair_required.lock().map_err(invalid)? {
            return Ok(());
        }
        let landed = self.landed_here.lock().map_err(invalid)?;
        if landed.is_empty() {
            return Err(invalid(
                "This call must make at least one byte-changing landing versus the pre-call snapshot: change a task record. Identical task bytes or restoring original task bytes do not count",
            ));
        }
        Ok(())
    }

    pub fn new_landing_count(&self) -> WorkflowResult<usize> {
        Ok(self.landed_here.lock().map_err(invalid)?.len())
    }

    /// Session receipts count this call's landed subjects. `assemble` and
    /// `compose` consume the complete retained-store count internally.
    pub fn compose_reply(&self, reply: &Value) -> WorkflowResult<Value> {
        if self.kind != RecordKind::Skeleton {
            return self.compose(reply);
        }
        let landed = self.new_landing_count()?;
        if !reply.as_object().is_some_and(|o| o.len() == 1)
            || reply.get("records_landed").and_then(Value::as_u64) != Some(landed as u64)
        {
            return Err(invalid(format!(
                "reply must be exactly {} (new landings in this call, excluding seeds)",
                json!({"records_landed":landed})
            )));
        }
        self.require_finding_repairs()?;
        self.compose(&json!({"records_landed":self.records()?.len()}))
    }

    /// Current stores use the durable per-call change set, including removals.
    /// Legacy stores fall back to full serialized records as evidence of work.
    pub fn has_landed_records(&self) -> WorkflowResult<bool> {
        if self.root.join("call-state.json").exists() {
            return Ok(self.new_landing_count()? > 0);
        }
        self.has_full_records()
    }

    fn has_full_records(&self) -> WorkflowResult<bool> {
        let snapshot = match &self.read_snapshot {
            Some(records) => records.clone(),
            None => self.saved_records()?,
        };
        for value in snapshot.into_values() {
            let record: StageRecord = serde_json::from_value(value.clone())?;
            if serde_json::to_value(record)? == value {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The artifact a `records_landed` reply stands for. A skeleton is
    /// composed whole from retained tasks; other kinds keep the reply and add
    /// landed findings and verification records. The count check still applies.
    pub fn compose(&self, reply: &Value) -> WorkflowResult<Value> {
        let mut assembled = self.assemble(reply)?;
        let tasks = assembled["tasks"].take();
        if self.kind == RecordKind::Skeleton {
            return Ok(json!({"schema_version":1,"acceptance_digest":"","tasks":tasks}));
        }
        let mut value = reply.clone();
        value["findings"] = assembled["findings"].take();
        if self.kind == RecordKind::Verify {
            value["verification_records"] = assembled["verification_records"].take();
        }
        if let Some(object) = value.as_object_mut() {
            object.remove("records_landed");
        }
        Ok(value)
    }

    /// The final reply the host can give for a session that ends without
    /// one. Only a skeleton qualifies: its artifact is the retained records
    /// themselves and its gate judges them; a review or verify verdict is the
    /// agent's own and is never invented here.
    pub fn retained_reply(&self) -> WorkflowResult<Option<Value>> {
        if self.kind != RecordKind::Skeleton {
            return Ok(None);
        }
        self.require_repaired()?;
        let landed = self.new_landing_count()?;
        // An identical seed re-land still supplies the active session's artifact
        // through compose_reply. Resume skips it: unchanged bytes prove no repair.
        if landed == 0 {
            return Ok(None);
        }
        self.require_finding_repairs()?;
        Ok(Some(json!({"records_landed": landed})))
    }
    /// Preserve valid retained tasks even when this call cannot complete.
    pub fn retained_artifact(&self) -> WorkflowResult<Option<Value>> {
        if self.kind != RecordKind::Skeleton
            || (!self.has_full_records()? && self.new_landing_count()? == 0)
        {
            return Ok(None);
        }
        let records = self.records()?;
        let tasks: Vec<_> = records
            .values()
            .filter_map(|record| record.task.as_ref())
            .collect();
        Ok(Some(
            json!({"schema_version":1,"acceptance_digest":"","tasks":tasks}),
        ))
    }

    /// Store-level completion for callers using `assemble`/`compose` directly.
    /// Sessions use `retained_reply`, which excludes seeded records.
    pub fn retained_completion(&self) -> WorkflowResult<Option<Value>> {
        if self.kind != RecordKind::Skeleton {
            return Ok(None);
        }
        self.require_repaired()?;
        let retained = self.records()?.len();
        Ok((retained > 0).then(|| json!({"records_landed":retained})))
    }
}
