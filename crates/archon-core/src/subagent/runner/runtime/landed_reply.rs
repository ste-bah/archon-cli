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
    let values = embedded_values(text)?;
    if values.len() == 1 && is_record(&values[0], tool) {
        return Ok(Some(values));
    }
    if values.iter().any(|v| contains_record(v, tool)) {
        return Err(
            "prose contains ambiguous record objects or arrays; send one record object or a reply that is only a JSON array".into(),
        );
    }
    Ok(None)
}

fn contains_record(value: &serde_json::Value, tool: &str) -> bool {
    is_record(value, tool)
        || value
            .as_array()
            .is_some_and(|items| items.iter().any(|item| contains_record(item, tool)))
}

fn embedded_values(text: &str) -> Result<Vec<serde_json::Value>, String> {
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(['{', '[']) {
        let candidate = &rest[start..];
        let next = candidate[1..].trim_start().chars().next();
        let json_like = match candidate.chars().next() {
            Some('{') => matches!(next, Some('"' | '}')),
            _ => matches!(next, Some('"' | '{' | '[' | ']' | '-' | '0'..='9')),
        };
        if !json_like {
            rest = &candidate[1..];
            continue;
        }
        let mut stream =
            serde_json::Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
        let value = stream.next().and_then(Result::ok).ok_or(
            "reply contains incomplete or malformed JSON; resend the records or final receipt",
        )?;
        rest = &candidate[stream.byte_offset()..];
        values.push(value);
    }
    Ok(values)
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
        let normalized = landing.normalize_reply(text);
        let normalized_text = normalized.as_ref().map(ToString::to_string);
        let record_text = if landing.tool_name() != "land-skeleton-record" {
            normalized_text.as_deref().unwrap_or(text)
        } else {
            text
        };
        let mut good_record = false;
        let error = match record_reply(record_text, landing.tool_name()) {
            Ok(Some(records)) => {
                let mut feedback = Vec::new();
                for record in records {
                    feedback.push(match landing.land(record) {
                        Ok(receipt) => {
                            good_record = true;
                            format!(
                                "host landed the record through {}. {receipt}",
                                landing.tool_name()
                            )
                        }
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
                let parsed = normalized.or_else(|| {
                    // A single embedded object can be a skeleton receipt; arrays
                    // cannot promote a nested object to a final receipt.
                    let mut values: Vec<_> = embedded_values(text)
                        .ok()?
                        .into_iter()
                        .filter(|v| v.is_object() && v.get("records_landed").is_some())
                        .collect();
                    if values.len() != 1 {
                        return None;
                    }
                    values.pop()
                });
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
                        Ok(()) => {
                            return Ok(LandedReply::Complete(parsed.as_ref().unwrap().to_string()));
                        }
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
            if good_record {
                if let Some(completion) = self.retained_landed_completion() {
                    return Ok(LandedReply::Complete(completion));
                }
                *incomplete = incomplete.saturating_add(1);
                return Ok(LandedReply::Retry(format!(
                    "{error}. Send the final receipt with records_landed now. {}",
                    landing.hint().unwrap_or_default()
                )));
            }
            if let Some(completion) = self.retained_landed_completion() {
                return Ok(LandedReply::Complete(completion));
            }
            return Err(
                archon_tools::audit_landing::IncompleteLandedArtifact(format!(
                    "{error}. {}",
                    landing.hint().unwrap_or_default()
                ))
                .into(),
            );
        }
        *incomplete = incomplete.saturating_add(1);
        Ok(LandedReply::Retry(format!(
            "Landed artifact incomplete: {error}. {}",
            landing.hint().unwrap_or_default()
        )))
    }
}
