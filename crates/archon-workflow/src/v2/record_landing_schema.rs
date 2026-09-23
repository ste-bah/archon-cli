//! The record contract as each landing kind sees it: the schema text the agent is shown, and the payload repair applied before deserialisation.
//!
//! One shared contract made every kind read the same field list, so a kind was
//! shown a field only its own path consumes and was then rejected for sending
//! it: strict deserialisation runs before the kind is considered, so a field
//! the record would have discarded still fails the whole landing. The contract
//! is therefore built per kind, and a payload is repaired per kind before it is
//! parsed - fields this kind never reads are dropped, and fields it does read
//! are decoded when they arrive JSON-encoded as a string.
use super::record_landing::{RecordKind, VERIFY_VERDICTS};
use crate::{
    WorkflowError, WorkflowResult, WorkflowV2CommandKind, WorkflowV2CommandStatus,
    WorkflowV2EvidenceKind, WorkflowV2Status,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::OnceLock;
/// Issue-39: the serialised variant names of every enum a `StageRecord` carries, in one place, so `schema_hint()` and the bridge JSON schema cannot drift.
pub struct EnumNames {
    pub evidence_kinds: Vec<String>,
    pub command_kinds: Vec<String>,
    pub command_statuses: Vec<String>,
    pub statuses: Vec<String>,
    pub verify_verdicts: Vec<String>,
}
pub fn enum_names() -> EnumNames {
    fn names<T: Serialize>(items: &[T]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| serde_json::to_value(i).ok()?.as_str().map(str::to_owned))
            .collect()
    }
    use {
        WorkflowV2CommandKind as C, WorkflowV2CommandStatus as X, WorkflowV2EvidenceKind as E,
        WorkflowV2Status as S,
    };
    EnumNames {
        evidence_kinds: names(&[
            E::Inspection,
            E::Implementation,
            E::Test,
            E::Review,
            E::Remediation,
            E::Blocker,
            E::Artifact,
            E::Other,
        ]),
        command_kinds: names(&[
            C::Inspect,
            C::Test,
            C::Build,
            C::Format,
            C::Review,
            C::Other,
        ]),
        command_statuses: names(&[X::Succeeded, X::Failed, X::Skipped]),
        statuses: names(&[
            S::Pending,
            S::Running,
            S::Accepted,
            S::Noop,
            S::Failed,
            S::Blocked,
            S::NeedsReview,
            S::Cancelled,
        ]),
        verify_verdicts: names(&VERIFY_VERDICTS),
    }
}
/// The record field carrying one skeleton entry. Only the skeleton path reads it; every other kind discards it.
const TASK: &str = "task";
/// The skeleton entry clause, appended to the skeleton contract only so no other kind is invited to send a field its path would discard.
const TASK_CLAUSE: &str = "task (skeleton records only): {task_id: TASK-<DOMAIN>-<NNN>, file_name: <task_id>.md, depends_on: [{task_id, consumes: [{artifact_path}], ordering_only: bool}]*, blocks: [task_id]*, implements: [PRD obligation id]*, deliverable_contracts: [{kind, artifact_path, min_instances: int}]*; implements and/or deliverable_contracts must be non-empty}, ";
/// The fields this kind never reads. Dropping them before deserialisation keeps a field the record would have discarded from failing the landing; every field the kind does read stays exactly as strict as it was.
pub(super) fn ignored_fields(kind: RecordKind) -> &'static [&'static str] {
    match kind {
        RecordKind::Skeleton => &[],
        RecordKind::Review | RecordKind::Verify => &[TASK],
    }
}
/// The object-valued fields this kind reads that are accepted JSON-encoded as a string as well as inline, because a model serialising a nested object into its enclosing field is a formatting slip, not a wrong value.
fn string_encoded_object_fields(kind: RecordKind) -> &'static [&'static str] {
    match kind {
        RecordKind::Skeleton => &[TASK],
        RecordKind::Review | RecordKind::Verify => &[],
    }
}
/// Repair one incoming payload for this kind: drop what the kind ignores, then decode any string-encoded object field it reads. Anything else is left untouched for the strict deserialiser to judge.
pub(super) fn prepare_payload(
    kind: RecordKind,
    object: &mut Map<String, Value>,
) -> WorkflowResult<()> {
    for field in ignored_fields(kind) {
        object.remove(*field);
    }
    for field in string_encoded_object_fields(kind) {
        let Some(Value::String(text)) = object.get(*field) else {
            continue;
        };
        let decoded: Value = serde_json::from_str(text).map_err(|e| {
            WorkflowError::ArtifactInvalid(format!(
                "record field `{field}` arrived as a string but is not valid JSON: {e}; send it as an object"
            ))
        })?;
        object.insert((*field).to_owned(), decoded);
    }
    Ok(())
}
/// Compact, exact shape of `StageRecord` for one landing kind; the variant lists are serialised from the real enums so a rename cannot drift from the text agents read, and a clause only one kind's path consumes appears only in that kind's text.
pub fn schema_hint(kind: RecordKind) -> &'static str {
    static SKELETON: OnceLock<String> = OnceLock::new();
    static VERIFY: OnceLock<String> = OnceLock::new();
    static REVIEW: OnceLock<String> = OnceLock::new();
    match kind {
        RecordKind::Skeleton => &SKELETON,
        RecordKind::Verify => &VERIFY,
        RecordKind::Review => &REVIEW,
    }
    .get_or_init(|| build(kind))
    .as_str()
}
fn build(kind: RecordKind) -> String {
    let (names, bar) = (enum_names(), |v: &[String]| v.join("|"));
    let task = if kind == RecordKind::Skeleton {
        TASK_CLAUSE
    } else {
        ""
    };
    format!(
        "{{subject: string (one of this call's subjects), findings: [object]* (each a non-empty claim|summary|finding|title; may carry task_id, file_name, severity, kind, evidence; when the subject is not a task each finding must name the task that owns the fix via task_id, or task_ids/canonical_task_ids, or set attributable_to_task:false), evidence: [{{kind: {evidence}, summary: string, source?: string}}]+, commands_run: [{{kind: {commands}, command: string, status: {command_statuses}, exit_code?: int, output_summary: string, pre_existing?: bool}}]*, status?: {statuses}, summary: string, {task}replace?: bool}}",
        evidence = bar(&names.evidence_kinds),
        commands = bar(&names.command_kinds),
        command_statuses = bar(&names.command_statuses),
        statuses = bar(&names.statuses)
    )
}
/// The top-level field names this kind may send, for the prose hint that accompanies the schema.
pub(super) fn field_list(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Skeleton => "{subject,evidence,commands_run,status,summary,task}",
        RecordKind::Review | RecordKind::Verify => {
            "{subject,findings,evidence,commands_run,status,summary}"
        }
    }
}
