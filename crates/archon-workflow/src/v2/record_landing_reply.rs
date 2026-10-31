//! Issue-550: the one final-answer contract of a call that lands records.
//!
//! A live skeleton author was shown two answer contracts at once — the whole
//! artifact and the landing protocol — and lost three of seven tries to the
//! mix: a count without the digest placeholder, a single record as the
//! answer, and a loop of re-landings. When records are landed the host owns
//! the artifact: it composes it from the retained records, every receipt ends
//! with the exact final reply, and a session that cannot give that reply can
//! still be completed from what it retained.
use super::record_landing::{RecordKind, RecordLanding, StageRecord, schema_hint};
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
        // Before the first landing there is no count to state yet.
        let last = if records.is_empty() {
            "Land each task as one record; every landing receipt ends with the exact final reply."
                .to_owned()
        } else {
            format!(
                "When no task is left to land, your final reply is exactly {}",
                json!({"records_landed": records.len()})
            )
        };
        Ok(format!(
            "Host retained {} records. Landed subjects: {}. Use {} with {}; each subject is a task id you establish, and landing a subject again replaces its task. Schema: {}. The host composes schema_version, acceptance_digest and tasks from the retained records: never send acceptance_digest, never send a record as your final reply. {last}",
            records.len(),
            serde_json::to_string(&records.keys().collect::<Vec<_>>())?,
            self.tool_name(),
            field_list(self.kind),
            schema_hint(self.kind),
        ))
    }

    /// The artifact a `records_landed` reply stands for. A skeleton is
    /// composed whole from the retained records; any other kind keeps the
    /// reply and takes its tasks from them. The count check still applies.
    pub fn compose(&self, reply: &Value) -> WorkflowResult<Value> {
        let tasks = self.assemble(reply)?["tasks"].take();
        if self.kind == RecordKind::Skeleton {
            return Ok(json!({"schema_version":1,"acceptance_digest":"","tasks":tasks}));
        }
        let mut value = reply.clone();
        value["tasks"] = tasks;
        if let Some(object) = value.as_object_mut() {
            object.remove("records_landed");
        }
        Ok(value)
    }

    /// The final reply the host can give for a session that ends without
    /// one. Only a skeleton qualifies: its artifact is the retained records
    /// themselves and its gate judges them; a review or verify verdict is the
    /// agent's own and is never invented here.
    pub fn retained_completion(&self) -> WorkflowResult<Option<Value>> {
        if self.kind != RecordKind::Skeleton {
            return Ok(None);
        }
        let retained = self.records()?.len();
        Ok((retained > 0).then(|| json!({"records_landed": retained})))
    }
}
