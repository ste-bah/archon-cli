//! Enforcement of the per-call output shape a workflow script DECLARED.
//!
//! A host call may declare what its result has to carry, via
//! `options.extra["outputs"]`. Until this module existed that declaration was
//! ADVICE ONLY: `call_data::agent_request` copies it into the prompt as a
//! constraint and nothing ever compared the returned result against it. A call
//! declaring `outputs: ["items"]` could return an ACCEPTED result containing no
//! items at all and be admitted unchanged, so the fan-out that reads
//! `data.items` downstream silently received zero work — a run that "succeeded"
//! having done nothing, with no error anywhere naming the empty output as the
//! cause.
//!
//! What is enforced is whatever the SCRIPT declared, never a fixed list of
//! names. Workflows are authored per objective and run against any language,
//! repository or toolchain, so a built-in required shape would be wrong for
//! every script that did not happen to match it. A call that declares nothing
//! is untouched — silence is not a request for a default shape.
//!
//! The rejection is returned as an ordinary `WorkflowV2AgentError` from
//! `parse_agent_output`, which is what feeds the EXISTING bounded repair loop:
//! the violation text is quoted back verbatim by `build_repair_prompt`, the
//! error's repair class (`Contract`) makes a repeat of the same violation
//! terminal after one re-ask, and the terminal state is the loop's own
//! `RepairExhausted`. Adding a second retry loop here would give a shape
//! violation a larger budget than every other contract breach.

use super::agent_adapter::{WorkflowV2AgentError, WorkflowV2AgentRequest};
use super::{WorkflowV2Result, WorkflowV2Status};

/// Reject an ACCEPTED result that does not carry the outputs its call declared.
///
/// Scoped to `Accepted` because that is the only status which CLAIMS the
/// declared work was done. `Blocked`, `Failed`, `Cancelled` and `NeedsReview`
/// report an honest non-outcome, and `Noop` claims the work was unnecessary;
/// none of them asserts the declared output exists. Demanding the data from
/// them would only teach agents to invent what they could not produce, which is
/// strictly worse than a visible non-outcome — an empty result that announces
/// itself is a fixable run, fabricated items are a corrupted one. The
/// dishonest-noop escape is already closed elsewhere and deliberately not
/// re-litigated here: `agent_adapter_a` refuses a noop without typed proof, and
/// `script::helpers_b` demotes an items-declaring noop that carries none.
///
/// Shape is not truth. A result that passes this has proved only that the
/// declared keys are present and non-empty; it is called last in
/// `validate_agent_result` so it can never stand in for the contracts that
/// judge the work itself.
pub(super) fn enforce_declared_call_outputs(
    request: &WorkflowV2AgentRequest,
    result: &WorkflowV2Result,
) -> Result<(), WorkflowV2AgentError> {
    let declared = declared_output_names(&request.call.options.extra);
    if declared.is_empty() || result.status != WorkflowV2Status::Accepted {
        return Ok(());
    }
    // Every violation, not the first. The same accounting that forced
    // `ImplementationAcceptedWithRequiredToolUnexercised` to report in bulk
    // applies verbatim: two missing outputs share the `Contract` repair class,
    // so `differs_from` can never buy the second one an extra attempt, and
    // naming them one per attempt would exhaust the budget before the agent had
    // been told the whole requirement.
    let violations = declared
        .iter()
        .filter_map(|name| output_violation(&result.data, name))
        .collect::<Vec<_>>();
    if violations.is_empty() {
        return Ok(());
    }
    Err(WorkflowV2AgentError::DeclaredOutputUnsatisfied {
        declared,
        violations,
    })
}

/// The output names this call declared, in whatever form the script wrote them.
///
/// An authored script is written by an LLM, so the same declaration arrives as
/// a list, as a bare string, or as an object keyed by name. All three name the
/// same thing and all three are read; anything else declares nothing rather
/// than being guessed at, because a misread declaration would enforce a shape
/// the script never asked for. Names are trimmed and de-duplicated so a sloppy
/// declaration cannot report the same violation twice in one message.
fn declared_output_names(
    extra: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<String> {
    let raw = match extra.get("outputs") {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>(),
        Some(serde_json::Value::String(value)) => vec![value.clone()],
        Some(serde_json::Value::Object(fields)) => fields.keys().cloned().collect(),
        _ => Vec::new(),
    };
    let mut names = Vec::new();
    for name in raw {
        let name = name.trim().to_string();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Why `data.<name>` fails the declaration, or `None` when it satisfies it.
///
/// Presence alone is not enough. The defect this exists to stop is an accepted
/// result whose declared key is present but carries nothing, which reads to
/// every downstream consumer exactly as the key being absent does. Emptiness is
/// therefore judged structurally — null, an empty array, an empty object, a
/// blank string — which holds for any declared name and any value a script
/// might ask for without knowing what the name means.
///
/// Types are deliberately NOT checked. The declaration is a list of names, not
/// a schema; inventing a type language here would put host assumptions back
/// inside a contract the script owns, and the host has no way to know that a
/// declared name should have been an array rather than a number.
///
/// The location is `data.<name>` and nothing else, because that is exactly what
/// the prompt constraint instructs and what every existing consumer reads. A
/// recursive search for the name would accept a result nested under the very
/// wrapper the constraint tells the agent not to build, so the violation the
/// agent is shown would not match the rule it was given.
fn output_violation(data: &serde_json::Value, name: &str) -> Option<String> {
    match data.get(name) {
        None => Some(format!("data.{name} is absent")),
        Some(serde_json::Value::Null) => Some(format!("data.{name} is null")),
        Some(serde_json::Value::Array(values)) if values.is_empty() => {
            Some(format!("data.{name} is an empty array"))
        }
        Some(serde_json::Value::Object(fields)) if fields.is_empty() => {
            Some(format!("data.{name} is an empty object"))
        }
        Some(serde_json::Value::String(value)) if value.trim().is_empty() => {
            Some(format!("data.{name} is an empty string"))
        }
        Some(_) => None,
    }
}
