//! Classify final replies while a host record landing capability is active.
use super::*;

pub(super) enum LandedReply {
    Final,
    Retry(String),
    Complete(String),
}

fn is_record(value: &serde_json::Value, tool: &str) -> bool {
    let key = if tool == "land-audit-record" {
        "declared_path"
    } else {
        "subject"
    };
    value.get(key).is_some_and(serde_json::Value::is_string)
        && value.get("records_landed").is_none()
        && value.get("data").is_none()
}

/// Recognise record containers before considering a retained completion.
/// Prose may contain exactly one complete object; ambiguous or broken JSON
/// must be retried, so it cannot silently discard new records.
fn record_reply(text: &str, tool: &str) -> Result<Option<Vec<serde_json::Value>>, String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        if is_record(&value, tool) {
            return Ok(Some(vec![value]));
        }
        if tool == "land-skeleton-record"
            && let Some(values) = value.as_array()
        {
            if !values.is_empty() && values.iter().all(|v| is_record(v, tool)) {
                return Ok(Some(values.clone()));
            }
            return Err("a record array must contain only landing record objects".into());
        }
        return Ok(None);
    }
    if tool != "land-skeleton-record" {
        return Ok(None);
    }
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(['{', '[']) {
        let mut stream =
            serde_json::Deserializer::from_str(&rest[start..]).into_iter::<serde_json::Value>();
        let value = stream.next().and_then(Result::ok).ok_or(
            "reply contains incomplete or malformed JSON; resend the records or final receipt",
        )?;
        rest = &rest[start + stream.byte_offset()..];
        values.push(value);
    }
    if values.is_empty() {
        return Ok(None);
    }
    if values.len() == 1 && is_record(&values[0], tool) {
        return Ok(Some(values));
    }
    Err(
        "prose must contain exactly one landing record object; use an array for multiple records"
            .into(),
    )
}

impl SubagentRunner {
    pub(super) fn retained_landed_completion(&self) -> Option<String> {
        let landing = self.tool_context.audit_landing.as_ref()?;
        let completion = landing.retained_completion()?;
        landing.complete(&completion).ok()?;
        tracing::warn!(completion = %completion,
            "session ends without a usable final reply; host completes it from retained records");
        Some(completion.to_string())
    }

    pub(super) fn complete_landed_or_error(
        &self,
        error: anyhow::Error,
    ) -> anyhow::Result<super::super::SubagentCompletion> {
        match self.retained_landed_completion() {
            Some(content) => Ok(completion_usage::from_stream(
                content,
                Some("end_turn".into()),
                self.progress.as_ref(),
            )),
            None => Err(error),
        }
    }

    pub(super) fn judge_landed_reply(
        &self,
        text: &str,
        incomplete: &mut u8,
    ) -> anyhow::Result<LandedReply> {
        let Some(landing) = &self.tool_context.audit_landing else {
            return Ok(LandedReply::Final);
        };
        let error = match record_reply(text, landing.tool_name()) {
            Ok(Some(records)) => {
                let mut feedback = Vec::new();
                for record in records {
                    feedback.push(match landing.land(record) {
                        Ok(receipt) => format!(
                            "host landed the record through {}. {receipt}",
                            landing.tool_name()
                        ),
                        Err(error) => format!("host could not land the record: {error}"),
                    });
                }
                format!(
                    "Your reply contained landing records, not the final reply; {}",
                    feedback.join("; ")
                )
            }
            Err(error) => error,
            Ok(None) => {
                let parsed = serde_json::from_str::<serde_json::Value>(text).ok();
                let compact = parsed.as_ref().and_then(|v| {
                    if landing.tool_name() == "land-audit-record" {
                        v.pointer("/data/repository_audit")
                    } else if landing.tool_name() == "land-skeleton-record" {
                        Some(v)
                    } else {
                        v.get("data")
                            .filter(|d| d.get("records_landed").is_some())
                            .or(Some(v))
                    }
                });
                match compact.filter(|v| v.get("records_landed").is_some()) {
                    Some(value) => match landing.complete(value) {
                        Ok(()) => return Ok(LandedReply::Final),
                        Err(error) => error,
                    },
                    None if landing.tool_name() == "land-audit-record" => {
                        return Ok(LandedReply::Final);
                    }
                    None => {
                        if landing.tool_name() == "land-skeleton-record"
                            && let Some(completion) = self.retained_landed_completion()
                        {
                            return Ok(LandedReply::Complete(completion));
                        }
                        "reply must include the landing receipt with records_landed".to_owned()
                    }
                }
            }
        };
        if *incomplete >= 2 {
            if let Some(completion) = self.retained_landed_completion() {
                return Ok(LandedReply::Complete(completion));
            }
            return Err(archon_tools::audit_landing::IncompleteLandedArtifact(error).into());
        }
        *incomplete += 1;
        Ok(LandedReply::Retry(format!(
            "Landed artifact incomplete: {error}. {}",
            landing.hint().unwrap_or_default()
        )))
    }
}
