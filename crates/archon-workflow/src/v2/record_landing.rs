//! Typed evidence records scoped to one host invocation, never repository writes.
use super::record_landing_merge::fold_landed_commands;
pub use super::record_landing_schema::{EnumNames, enum_names, schema_hint};
use super::record_landing_schema::{field_list, prepare_payload};
use crate::{
    WorkflowError, WorkflowResult, WorkflowV2AgentRequest, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2Result, WorkflowV2Status,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecordKind {
    Review,
    Verify,
    Skeleton,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRecord {
    subject: String,
    #[serde(default)]
    findings: Vec<Value>,
    #[serde(default)]
    evidence: Vec<WorkflowV2Evidence>,
    #[serde(default)]
    commands_run: Vec<WorkflowV2CommandRecord>,
    #[serde(default)]
    status: Option<WorkflowV2Status>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    task: Option<crate::task_skeleton::FrozenTask>,
    /// Issue-36: a write-time instruction, never persisted state. When true the record supersedes the earlier landing for its subject instead of unioning with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    replace: bool,
}
/// Issue-39: the verdicts a verify record may land; `validate` and the tool schema both read this list so the enum the model sees is the set the host accepts.
pub const VERIFY_VERDICTS: [WorkflowV2Status; 5] = [
    WorkflowV2Status::Accepted,
    WorkflowV2Status::Noop,
    WorkflowV2Status::Failed,
    WorkflowV2Status::Blocked,
    WorkflowV2Status::NeedsReview,
];
/// Issue-35: live wf-719ff3b0 rejected ~30 landings with only `missing field kind`, so the reducer landed probe records to discover the shape. Name the path and the schema in one string.
fn describe(err: serde_path_to_error::Error<serde_json::Error>, kind: RecordKind) -> WorkflowError {
    let (path, inner) = (err.path().to_string(), err.inner().to_string());
    let field = inner
        .strip_prefix("missing field `")
        .and_then(|s| s.split('`').next());
    let at = match (path.as_str(), field) {
        (".", Some(f)) => f.to_owned(),
        (".", None) => "<record>".to_owned(),
        (p, Some(f)) => format!("{p}.{f}"),
        (p, None) => p.to_owned(),
    };
    invalid(format!(
        "invalid record at {at}: {inner}. Expected schema: {}",
        schema_hint(kind)
    ))
}
fn with_schema(err: WorkflowError, kind: RecordKind) -> WorkflowError {
    match err {
        WorkflowError::ArtifactInvalid(m) => {
            invalid(format!("{m}. Expected schema: {}", schema_hint(kind)))
        }
        other => other,
    }
}
pub struct RecordLanding {
    root: PathBuf,
    kind: RecordKind,
    subjects: Vec<String>,
    subjects_are_tasks: bool,
    lock: Mutex<()>,
}
fn invalid(e: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::ArtifactInvalid(e.to_string())
}
tokio::task_local! { static RECORDS: Arc<RecordLanding>; }
pub async fn scope<T>(
    records: Arc<RecordLanding>,
    future: impl std::future::Future<Output = T>,
) -> T {
    RECORDS.scope(records, future).await
}
pub fn current() -> Option<Arc<RecordLanding>> {
    RECORDS.try_with(Clone::clone).ok()
}
impl RecordLanding {
    /// `subjects_are_tasks` says whether each subject is a canonical task id (map calls) or an opaque call id (reduce calls); it decides whether the subject may stand in for a finding's task.
    pub fn open(
        root: PathBuf,
        identity: String,
        kind: RecordKind,
        subjects: Vec<String>,
        subjects_are_tasks: bool,
    ) -> WorkflowResult<Self> {
        std::fs::create_dir_all(&root).map_err(invalid)?;
        let path = root.join("contract.json");
        let expected = json!({"identity":identity,"kind":kind,"subjects":subjects});
        if path.exists() {
            let saved: Value = serde_json::from_slice(&std::fs::read(&path).map_err(invalid)?)?;
            if saved != expected {
                return Err(invalid("record landing identity changed"));
            }
        } else {
            write(&path, &expected)?;
        }
        Ok(Self {
            root,
            kind,
            subjects,
            subjects_are_tasks,
            lock: Mutex::new(()),
        })
    }
    pub fn kind(&self) -> RecordKind {
        self.kind
    }
    pub fn tool_name(&self) -> &'static str {
        match self.kind {
            RecordKind::Review => "land-review-record",
            RecordKind::Verify => "land-verify-record",
            RecordKind::Skeleton => "land-skeleton-record",
        }
    }
    fn validate(&self, r: &StageRecord) -> WorkflowResult<()> {
        if r.subject.trim().is_empty() || r.subject.len() > 4096 {
            return Err(invalid("record subject is missing or too long"));
        }
        if self.kind != RecordKind::Skeleton && !self.subjects.contains(&r.subject) {
            return Err(invalid("record subject is outside this call"));
        }
        if self.kind == RecordKind::Skeleton {
            let task = r
                .task
                .as_ref()
                .ok_or_else(|| invalid("skeleton record requires task"))?;
            if task.task_id != r.subject || task.file_name.is_empty() {
                return Err(invalid("skeleton task identity mismatch"));
            }
            crate::repository_audit::contract::validate_path(&task.file_name).map_err(invalid)?;
            let skeleton = crate::task_skeleton::TaskSkeleton {
                schema_version: 1,
                acceptance_digest: "pending".into(),
                tasks: vec![task.clone()],
            };
            crate::task_skeleton::validate_skeleton(&skeleton, "pending").map_err(invalid)?;
            // Issue-38: live wf-f29c0e96 landed 15 `{task_id,file_name}` stubs that passed here, so the host froze a skeleton with zero obligations and the judge raised 126 findings; a task naming nothing it implements or delivers is unownable.
            if task.implements.is_empty() && task.deliverable_contracts.is_empty() {
                return Err(invalid(
                    "a skeleton task must name the PRD obligations it implements and/or the artifacts it delivers; a stub with neither cannot be owned or verified",
                ));
            }
            // Issue-39: a blank obligation id or a contract with no kind/path passed here and was silently skipped downstream, so the task looked owned while owing nothing; name the exact hollow entry.
            if let Some(i) = task.implements.iter().position(|id| id.trim().is_empty()) {
                return Err(invalid(format!(
                    "task.implements[{i}] is blank; each entry must be a PRD obligation id"
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
            return Ok(());
        }
        if r.evidence.is_empty() || r.evidence.iter().any(|e| e.summary.trim().is_empty()) {
            return Err(invalid("record requires concrete evidence"));
        }
        if serde_json::to_vec(r)?.len() > 32768 {
            return Err(invalid(
                "record exceeds 32768 bytes; keep findings and evidence concise",
            ));
        }
        if r.findings.len() > 25
            || r.findings.iter().any(|f| {
                !f.is_object()
                    || !["claim", "summary", "finding", "title"].iter().any(|key| {
                        f.get(key)
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.trim().is_empty())
                    })
            })
        {
            return Err(invalid(
                "findings must be at most 25 structured findings with a nonempty claim/summary/finding/title",
            ));
        }
        if self.kind == RecordKind::Verify
            && !r.status.is_some_and(|s| VERIFY_VERDICTS.contains(&s))
        {
            return Err(invalid("verify record needs an explicit terminal verdict"));
        }
        // Issue-39: an accepted/noop verify record with no succeeded command landed and was only demoted after the session ended; a pass must carry the command that proved it, and a synthesised output_summary is not a capture.
        if self.kind == RecordKind::Verify
            && matches!(
                r.status,
                Some(WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
            )
            && !r.commands_run.iter().any(|c| {
                c.status == WorkflowV2CommandStatus::Succeeded
                    && !c.command.trim().is_empty()
                    && captured(&c.output_summary)
            })
        {
            return Err(invalid(
                "verify record with status accepted/noop requires at least one succeeded commands_run entry with a captured output_summary",
            ));
        }
        // Issue-37: the live reduce record named tasks only in prose titles, so nothing downstream could route its findings; when the subject is a call id every finding must say which task owns the fix, or disclaim ownership.
        if self.kind == RecordKind::Review
            && !self.subjects_are_tasks
            && r.findings.iter().any(|f| {
                finding_task_ids(f).is_none()
                    && f.get("attributable_to_task").and_then(Value::as_bool) != Some(false)
            })
        {
            return Err(invalid(
                "each finding must name the task that owns the fix (task_id, or task_ids/canonical_task_ids) or set attributable_to_task:false when no single task can act on it",
            ));
        }
        Ok(())
    }
    pub fn land(&self, mut value: Value) -> WorkflowResult<()> {
        let _lock = self.lock.lock().map_err(invalid)?;
        // Issue-39: the agent envelope fills a missing command kind, derives status from exit_code and synthesises output_summary, while a landed record rejected the same entry with `missing field kind`; apply the one normaliser here so the two paths agree, and let the Issue-35 path error name whatever it cannot fill.
        if let Some(object) = value.as_object_mut() {
            super::agent_output_normalize::normalize_commands(object);
            // A field this kind never reads must not be able to fail its landing, and a field it does read is accepted string-encoded; both are settled before the strict parse, which cannot see the kind.
            prepare_payload(self.kind, object).map_err(|e| with_schema(e, self.kind))?;
        }
        let mut record: StageRecord =
            serde_path_to_error::deserialize(value).map_err(|e| describe(e, self.kind))?;
        self.validate(&record)
            .map_err(|e| with_schema(e, self.kind))?;
        use sha2::{Digest, Sha256};
        let path = self.root.join(format!(
            "record-{:x}.json",
            Sha256::digest(record.subject.as_bytes())
        ));
        let replace = std::mem::take(&mut record.replace);
        if path.exists() && self.kind != RecordKind::Skeleton && !replace {
            let previous: StageRecord =
                serde_json::from_slice(&std::fs::read(&path).map_err(invalid)?)?;
            self.validate(&previous)?;
            for finding in previous.findings {
                if !record.findings.contains(&finding) {
                    record.findings.push(finding);
                }
            }
            for evidence in previous.evidence {
                if !record.evidence.contains(&evidence) {
                    record.evidence.push(evidence);
                }
            }
            fold_landed_commands(&mut record.commands_run, previous.commands_run);
            if matches!(
                previous.status,
                Some(
                    WorkflowV2Status::Failed
                        | WorkflowV2Status::Blocked
                        | WorkflowV2Status::NeedsReview
                )
            ) {
                record.status = previous.status;
                if !previous.summary.is_empty() {
                    record.summary = format!("{}; {}", previous.summary, record.summary);
                }
            }
            self.validate(&record)
                .map_err(|e| with_schema(e, self.kind))?;
        }
        write(&path, &record)
    }
    fn records(&self) -> WorkflowResult<BTreeMap<String, StageRecord>> {
        let mut records = BTreeMap::new();
        for entry in std::fs::read_dir(&self.root).map_err(invalid)? {
            let path = entry.map_err(invalid)?.path();
            if !path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|s| s.starts_with("record-") && s.ends_with(".json"))
            {
                continue;
            }
            let record: StageRecord =
                serde_json::from_slice(&std::fs::read(path).map_err(invalid)?)?;
            self.validate(&record)?;
            if records.insert(record.subject.clone(), record).is_some() {
                return Err(invalid("duplicate persisted record subject"));
            }
        }
        Ok(records)
    }
    pub fn remaining(&self) -> WorkflowResult<Vec<String>> {
        let records = self.records()?;
        Ok(self
            .subjects
            .iter()
            .filter(|id| !records.contains_key(*id))
            .cloned()
            .collect())
    }
    pub fn hint(&self) -> WorkflowResult<String> {
        let records = self.records()?;
        Ok(format!(
            "Host retained {} records. Landed subjects: {}. Remaining subjects: {}. Use {} with {}. Land an empty findings array plus evidence for a checked clean subject. Final data.records_landed must equal the complete saved count. Do not re-gather landed subjects. Skeleton subjects are task ids you establish; final raw skeleton may contain records_landed instead of tasks. All original final validation still applies. Re-landing a subject unions with the earlier record; send replace:true to supersede it (use this to withdraw a finding). Schema: {}",
            records.len(),
            serde_json::to_string(&records.keys().collect::<Vec<_>>())?,
            serde_json::to_string(&self.remaining()?)?,
            self.tool_name(),
            field_list(self.kind),
            schema_hint(self.kind)
        ))
    }
    pub fn assemble(&self, completion: &Value) -> WorkflowResult<Value> {
        let _lock = self.lock.lock().map_err(invalid)?;
        let records = self.records()?;
        let missing = self
            .subjects
            .iter()
            .filter(|id| !records.contains_key(*id))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(invalid(format!("missing record subjects: {missing:?}")));
        }
        // Issue-39: the bare "does not match" left the agent guessing which side was wrong; say both numbers.
        let given = completion.get("records_landed").and_then(Value::as_u64);
        if given != Some(records.len() as u64) {
            return Err(invalid(format!(
                "records_landed={} but host retained {} record(s)",
                given.map_or("missing".to_owned(), |n| n.to_string()),
                records.len()
            )));
        }
        let rows = records.values().collect::<Vec<_>>();
        // Issue-37: reduce findings were stamped with the call id as their task, so the script grouped them under "adversarial-review-reduce" and skipped every remediation; the subject stands in only when it is a task.
        let findings = rows
            .iter()
            .flat_map(|r| {
                r.findings.iter().cloned().map(|mut finding| {
                    let ids = finding_task_ids(&finding)
                        .or_else(|| self.subjects_are_tasks.then(|| json!([r.subject])));
                    if let (Some(object), Some(ids)) = (finding.as_object_mut(), ids) {
                        object.insert("canonical_task_ids".into(), ids);
                    }
                    finding
                })
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"findings":findings,"verification_records":rows,"tasks":rows.iter().filter_map(|r|r.task.as_ref()).collect::<Vec<_>>()}),
        )
    }
    pub fn expand(&self, result: &mut WorkflowV2Result) -> WorkflowResult<()> {
        if result.data.get("records_landed").is_none() {
            return Ok(());
        }
        let data = self.assemble(&result.data)?;
        result.data["findings"] = data["findings"].clone();
        if self.kind == RecordKind::Verify {
            result.data["verification_records"] = data["verification_records"].clone();
        }
        for row in data["verification_records"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let record: StageRecord = serde_json::from_value(row.clone())?;
            result.evidence.extend(record.evidence);
            fold_landed_commands(&mut result.commands_run, record.commands_run);
            if self.kind == RecordKind::Verify
                && matches!(
                    record.status,
                    Some(
                        WorkflowV2Status::Failed
                            | WorkflowV2Status::Blocked
                            | WorkflowV2Status::NeedsReview
                    )
                )
            {
                result.status = WorkflowV2Status::NeedsReview;
                result.residual_gaps.push(crate::WorkflowV2ResidualGap {
                    id: record.subject.clone(),
                    description: format!(
                        "Retained verification failure for {}: {}",
                        record.subject, record.summary
                    ),
                    severity: Some("blocking".into()),
                });
            }
        }
        if result.summary.trim().is_empty() {
            result.summary = "Host assembled landed evidence records".into();
        }
        Ok(())
    }
}
/// Issue-39: an `output_summary` counts as captured only when it is non-blank and not the envelope normaliser's placeholder.
pub(super) fn captured(output_summary: &str) -> bool {
    !output_summary.trim().is_empty()
        && !output_summary
            .starts_with(super::agent_output_normalize::SYNTHESIZED_OUTPUT_SUMMARY_PREFIX)
}
/// The task ids a finding names for itself: `canonical_task_ids`, else `task_ids`, else `[task_id]`; `None` when it names no task.
fn finding_task_ids(finding: &Value) -> Option<Value> {
    let nonempty = |key: &str| {
        finding
            .get(key)
            .and_then(Value::as_array)
            .filter(|a| {
                !a.is_empty()
                    && a.iter()
                        .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
            })
            .map(|a| Value::Array(a.clone()))
    };
    nonempty("canonical_task_ids")
        .or_else(|| nonempty("task_ids"))
        .or_else(|| {
            finding
                .get("task_id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(|s| json!([s]))
        })
}
fn write(path: &std::path::Path, value: &impl Serialize) -> WorkflowResult<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, serde_json::to_vec(value)?).map_err(invalid)?;
    std::fs::rename(tmp, path).map_err(invalid)
}
pub(crate) fn expand_result(
    _request: &WorkflowV2AgentRequest,
    result: &mut WorkflowV2Result,
) -> Result<(), crate::WorkflowV2AgentError> {
    if let Some(records) = current() {
        records
            .expand(result)
            .map_err(|e| crate::WorkflowV2AgentError::InvalidResult(e.to_string()))?;
    }
    Ok(())
}
