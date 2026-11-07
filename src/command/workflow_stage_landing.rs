//! Host bridge for typed single-artifact stage evidence.
use archon_workflow::{
    v2::record_landing::{RecordKind, RecordLanding},
    *,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn prepare(
    request: &mut WorkflowV2AgentRequest,
    store: Option<&WorkflowV2ResultStore>,
) -> WorkflowResult<Option<Arc<RecordLanding>>> {
    let Some(store) = store else { return Ok(None) };
    if request.is_write_capable()
        || request
            .call
            .options
            .extra
            .contains_key("repository_audit_contract")
    {
        return Ok(None);
    }
    let extra = &request.call.options.extra;
    let kind = if extra.get("recordLanding").and_then(Value::as_str) == Some("skeleton") {
        RecordKind::Skeleton
    } else if extra.contains_key("reviewContract") || extra.contains_key("review_contract") {
        RecordKind::Review
    } else if request.call.options.item_kind.as_deref() == Some("focused_verification")
        || request.input.get("item_kind").and_then(Value::as_str) == Some("focused_verification")
        || request.input.get("focused_verification").is_some()
        || request
            .input
            .pointer("/item/focused_verification")
            .is_some()
    {
        RecordKind::Verify
    } else {
        return Ok(None);
    };
    let mut subjects =
        archon_workflow::v2::branch_stamping::branch_canonical_task_ids(&request.input);
    // Issue-37: a reduce call has no canonical task ids, so its subject is the call id and must not be stamped onto findings as their task.
    let subjects_are_tasks = kind != RecordKind::Skeleton && !subjects.is_empty();
    if kind == RecordKind::Skeleton {
        subjects.clear();
    } else if subjects.is_empty() {
        subjects.push(request.call.id.clone());
    }
    subjects.sort();
    subjects.dedup();
    use sha2::{Digest, Sha256};
    let identity = format!("{:x}", Sha256::digest(serde_json::to_vec(request)?));
    let root = store
        .root()
        .join("stage-records")
        .join(&identity)
        .join(uuid::Uuid::new_v4().to_string());
    let records = Arc::new(RecordLanding::open(
        root.clone(),
        identity,
        kind,
        subjects,
        subjects_are_tasks,
    )?);
    crate::command::workflow_task_set::reauthor::cache::write_atomic(
        &root.join("author.json"),
        &serde_json::to_vec(&request.call.id)?,
    )
    .map_err(|error| WorkflowError::ArtifactInvalid(error.to_string()))?;
    if kind == RecordKind::Skeleton {
        if let Some(previous) = extra
            .get("previousSkeleton")
            .filter(|value| !value.is_null())
        {
            let previous = match previous.as_str() {
                Some(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                None => previous.clone(),
            };
            records.seed_skeleton(
                &previous,
                extra.get("frozenSkeleton").and_then(Value::as_bool) == Some(true),
            )?;
        }
        records.require_finding_landings(
            extra
                .get("openFindings")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )?;
    }
    request.task.push_str(&format!("\n{}", records.hint()?));
    Ok(Some(records))
}
/// Enforce the landing contract at raw dispatch too; alternate clients cannot
/// turn prose into candidate bytes by bypassing the runner.
pub(super) fn compose_raw_reply(records: &RecordLanding, text: &str) -> WorkflowResult<String> {
    let parsed = serde_json::from_str::<Value>(text);
    if records.kind() != RecordKind::Skeleton
        && parsed.as_ref().ok().is_none_or(|value| {
            value.get("records_landed").is_none() && value.pointer("/data/records_landed").is_none()
        })
    {
        return Ok(text.to_owned());
    }
    let artifact = match parsed {
        Ok(mut value) => {
            if records.kind() != RecordKind::Skeleton
                && value.pointer("/data/records_landed").is_some()
            {
                let mut result: WorkflowV2Result = serde_json::from_value(value.clone())?;
                records.expand(&mut result)?;
                value["data"] = records.compose_reply(&value["data"])?;
                let expanded = serde_json::to_value(result)?;
                for field in [
                    "status",
                    "summary",
                    "evidence",
                    "commands_run",
                    "residual_gaps",
                ] {
                    value[field] = expanded[field].clone();
                }
                Ok(value)
            } else {
                if records.kind() == RecordKind::Verify {
                    let mut result = WorkflowV2Result {
                        data: value.clone(),
                        ..Default::default()
                    };
                    records.expand(&mut result)?;
                    if result.status == WorkflowV2Status::NeedsReview {
                        return Err(WorkflowError::ArtifactInvalid(
                            "retained verification failure requires a result envelope".into(),
                        ));
                    }
                }
                records.compose_reply(&value)
            }
        }
        Err(error) => Err(WorkflowError::ArtifactInvalid(error.to_string())),
    };
    let artifact = artifact.or_else(|error| match records.retained_reply()? {
        Some(completion) => records.compose_reply(&completion),
        None => Err(error),
    })?;
    Ok(serde_json::to_string(&artifact)?)
}
/// A contract refusal keeps the call's valid landings available to its next author.
pub(super) fn refused_reply(
    call_id: &str,
    error: WorkflowError,
    records: Option<&RecordLanding>,
) -> WorkflowV2Result {
    let mut result = archon_workflow::v2::host_fault::invalid_answer_result(call_id, error);
    if let Some(records) = records {
        match records.retained_artifact() {
            Ok(Some(artifact)) => {
                result.data["content"] = json!(artifact.to_string());
                result.data["retainedLandings"] = json!(true);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "could not read retained landing artifact"),
        }
    }
    result
}
struct Bridge(Arc<RecordLanding>);
impl archon_tools::audit_landing::LandingHost for Bridge {
    fn accepts_record_arrays(&self) -> bool {
        self.0.kind().host_composes_artifact()
    }
    fn empty_reply_policy(&self) -> archon_tools::audit_landing::EmptyReplyPolicy {
        use archon_tools::audit_landing::EmptyReplyPolicy;
        if self.0.kind().host_composes_artifact() {
            EmptyReplyPolicy::RetainedCompletion
        } else {
            EmptyReplyPolicy::Resend
        }
    }
    fn receipt_envelope_pointer(&self) -> &'static str {
        if self.0.kind().host_composes_artifact() {
            ""
        } else {
            "/data"
        }
    }
    fn progress_key_fields(&self) -> &'static [&'static str] {
        &["subject"]
    }
    fn allows_unreceipted_reply(&self) -> bool {
        false
    }
    fn normalize_record_reply(&self) -> bool {
        !self.0.kind().host_composes_artifact()
    }

    fn normalize_reply(&self, text: &str) -> Option<Value> {
        archon_workflow::v2::parse_landing_envelope(text)
    }
    fn tool_name(&self) -> &'static str {
        self.0.tool_name()
    }
    // Issue-39: one schema for all three kinds said only `subject` was required and left every item a bare object, so the model learned the contract by being rejected; build it per kind from the enums `validate` enforces.
    fn schema(&self) -> Option<Value> {
        let (kind, names) = (
            self.0.kind(),
            archon_workflow::v2::record_landing::enum_names(),
        );
        let required = match kind {
            RecordKind::Review => json!(["subject", "evidence", "summary"]),
            RecordKind::Verify => json!(["subject", "evidence", "summary", "status"]),
            RecordKind::Skeleton => json!(["subject", "evidence", "summary"]),
        };
        let mut properties = json!({
            "subject":{"type":"string","description":"One of this call's subjects"},
            "evidence":{"type":"array","minItems":1,"items":{"type":"object","required":["kind","summary"],"properties":{"kind":{"type":"string","enum":names.evidence_kinds},"summary":{"type":"string"},"source":{"type":"string"}}}},
            "commands_run":{"type":"array","items":{"type":"object","required":["kind","command","status","output_summary"],"properties":{
                "kind":{"type":"string","enum":names.command_kinds},"command":{"type":"string"},"status":{"type":"string","enum":names.command_statuses},
                "exit_code":{"type":"integer"},"output_summary":{"type":"string","description":"Captured output, not a placeholder"},"pre_existing":{"type":"boolean"}}}},
            "status":{"type":"string","enum":names.verify_verdicts},"summary":{"type":"string"},
            "replace":{"type":"boolean","description":"Supersede the earlier record for this subject instead of unioning with it"}});
        if kind == RecordKind::Verify {
            properties["status"]["description"] = json!(
                "Terminal verdict; accepted/noop needs a succeeded commands_run entry with captured output"
            );
        }
        if kind != RecordKind::Skeleton {
            properties["findings"] = json!({"type":"array","maxItems":25,"items":{"type":"object","anyOf":[{"required":["claim"]},{"required":["summary"]},{"required":["finding"]},{"required":["title"]}],
            "properties":{"claim":{"type":"string"},"summary":{"type":"string"},"finding":{"type":"string"},"title":{"type":"string"},"task_id":{"type":"string"},"task_ids":{"type":"array","items":{"type":"string"}},"canonical_task_ids":{"type":"array","items":{"type":"string"}},"attributable_to_task":{"type":"boolean"},"file_name":{"type":"string"},"severity":{"type":"string"},"kind":{"type":"string"},"evidence":{}}}});
        }
        // Issue-38: `FrozenTask` deserialises with serde defaults and accepts unknown keys, so `task` stays open; the listed properties are the fields the host reads.
        if kind == RecordKind::Skeleton {
            properties["task"] = archon_workflow::v2::record_landing::task_schema();
            properties["remove"] =
                json!({"type":"boolean","description":"Remove a draft task by subject; omit task"});
        }
        Some(
            json!({"type":"object","required":required,"additionalProperties":false,"description":archon_workflow::v2::record_landing::schema_hint(kind),"properties":properties}),
        )
    }
    fn land(&self, value: Value) -> Result<String, String> {
        self.0.land(value).map_err(|e| e.to_string())?;
        self.hint()
    }
    fn hint(&self) -> Result<String, String> {
        self.0.hint().map_err(|e| e.to_string())
    }
    fn complete(&self, value: &Value) -> Result<(), String> {
        self.0
            .compose_reply(value)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn retained_completion(&self) -> Option<Value> {
        self.0.retained_reply().ok().flatten()
    }
}
pub(crate) async fn scope<T>(
    records: Option<Arc<RecordLanding>>,
    work: impl std::future::Future<Output = T>,
) -> T {
    if let Some(records) = records {
        let capability = Arc::new(archon_tools::audit_landing::AuditLanding::new(
            Arc::new(Bridge(records.clone())),
            None,
        ));
        archon_workflow::v2::record_landing::scope(
            records,
            archon_tools::audit_landing::scope(capability, work),
        )
        .await
    } else {
        work.await
    }
}

#[cfg(test)]
#[path = "workflow_stage_landing_host_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_stage_landing_p5_tests.rs"]
mod p5_tests;

#[cfg(test)]
#[path = "workflow_stage_landing_reply_tests.rs"]
mod reply_tests;

#[cfg(test)]
#[path = "workflow_landing_corpus_tests.rs"]
mod corpus_tests;
