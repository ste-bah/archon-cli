//! The final reply of a session that lands records through a host landing
//! capability (audit or stage records).
//!
//! Issue-550: a live skeleton author answered with one landing record instead
//! of the final reply, and another lost its call to "incomplete landed
//! artifact" after the loop guard ended it with every record retained. A
//! record given as the final reply is landed like any other; a session that
//! would fail while the host can complete it from its retained records ends
//! with that completion, which the host's own gate then judges.
use super::*;

pub(super) enum LandedReply {
    /// Not a landing reply, or a complete one: return the text as it is.
    Final,
    /// Ask again with this feedback; the session continues.
    Retry(String),
    /// End the session with this host-composed final reply.
    Complete(String),
}

/// A final reply that is one landing record, not the call's final reply.
fn is_record(value: &serde_json::Value) -> bool {
    value
        .get("subject")
        .is_some_and(serde_json::Value::is_string)
        && value.get("records_landed").is_none()
        && value.get("data").is_none()
}

impl SubagentRunner {
    /// The final reply the host can give from retained records alone.
    pub(super) fn retained_landed_completion(&self) -> Option<String> {
        let landing = self.tool_context.audit_landing.as_ref()?;
        let completion = landing.retained_completion()?;
        landing.complete(&completion).ok()?;
        tracing::warn!(
            completion = %completion,
            "session ends without a usable final reply; host completes it from retained records"
        );
        Some(completion.to_string())
    }

    pub(super) fn judge_landed_reply(
        &self,
        text: &str,
        incomplete: &mut u8,
    ) -> anyhow::Result<LandedReply> {
        let Some(landing) = &self.tool_context.audit_landing else {
            return Ok(LandedReply::Final);
        };
        let parsed = serde_json::from_str::<serde_json::Value>(text);
        let error = match parsed.as_ref().ok() {
            Some(record) if is_record(record) => match landing.land(record.clone()) {
                Ok(receipt) => {
                    return Ok(LandedReply::Retry(format!(
                        "Your reply was one landing record, not the final reply; the host landed it through {}. {receipt}",
                        landing.tool_name()
                    )));
                }
                Err(error) => format!(
                    "your reply was one landing record, not the final reply, and the host could not land it: {error}"
                ),
            },
            parsed => {
                let compact = parsed.and_then(|v| {
                    if landing.tool_name() == "land-audit-record" {
                        v.pointer("/data/repository_audit")
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
                    None => return Ok(LandedReply::Final),
                }
            }
        };
        if *incomplete >= 2 {
            if let Some(completion) = self.retained_landed_completion() {
                return Ok(LandedReply::Complete(completion));
            }
            anyhow::bail!("incomplete landed artifact: {error}");
        }
        *incomplete += 1;
        Ok(LandedReply::Retry(format!(
            "Landed artifact incomplete: {error}. {}",
            landing.hint().unwrap_or_default()
        )))
    }
}
