use serde_json::{Map, Value};

use super::WorkflowV2AgentRequest;

pub(super) fn normalize_agent_output(
    request: &WorkflowV2AgentRequest,
    output: &str,
) -> serde_json::Result<Value> {
    let mut value: Value = parse_envelope_document(output)?;
    let Some(object) = value.as_object_mut() else {
        return Ok(value);
    };
    stamp_envelope(request, object);
    normalize_path_records(object, "artifacts");
    normalize_path_records(object, "files_read");
    normalize_path_records(object, "files_changed");
    stamp_artifact_ids(object);
    normalize_commands(object);
    Ok(value)
}

/// Parse the agent reply as one JSON envelope. Providers routinely wrap an
/// otherwise-valid envelope in markdown fences or prose; when the whole reply
/// is not bare JSON, take the LAST complete top-level object that looks like a
/// result envelope. Location only — no content is invented, the forbidden-text
/// guard has already seen the full raw reply, and every schema and validation
/// gate still runs on whatever parses here.
///
/// Last-wins is not a guess: agent output is sequential, so in both observed
/// multi-object shapes — an echoed schema example followed by the real
/// envelope, and a fenced draft followed by "now as pure JSON" and the final
/// envelope — the reply the agent means is the one it wrote last. Rejecting
/// instead (the previous behavior) failed branches whose final envelope was
/// complete and valid, live: a verify branch drafted its envelope, restated it
/// verbatim, and was binned as "expected value at line 1 column 1".
fn parse_envelope_document(output: &str) -> serde_json::Result<Value> {
    let root_error = match serde_json::from_str(output.trim()) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let mut last_envelope: Option<Value> = None;
    let mut skip_until = 0;
    for (index, _) in output.match_indices(['{', '[']) {
        if index < skip_until {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&output[index..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(value)) => {
                skip_until = index + stream.byte_offset();
                // A complete array is skipped, not fatal. `raw_decode`
                // consumed the whole array and `skip_until` is now past it, so
                // an envelope nested inside it is never seen as a top-level
                // document — the "a one-element array must not impersonate the
                // envelope" rule holds by construction, without aborting.
                //
                // This used to `return Err`, which was harmless while ANY
                // second document also aborted the parse. Once the multi-object
                // rule became "take the last envelope", the array arm was the
                // only hard exit left in the loop, so a bracketed list written
                // in an agent's prose preamble killed a reply whose envelope
                // was complete and valid a few lines later. Observed live on a
                // shape-repair call.
                if value.is_array() {
                    continue;
                }
                // Every envelope declares `status`; evidence items lack it,
                // and task_coverage entries are told apart inside
                // `is_result_envelope`. Non-envelope objects (echoed
                // examples, prose-adjacent fragments) are skipped, not fatal.
                if is_result_envelope(&value) {
                    last_envelope = Some(value);
                }
            }
            // An unterminated object is a truncation signature: the reply is
            // structurally incomplete, and any complete object inside it (an
            // echoed branch envelope in data.items, a coverage entry) could
            // impersonate the real reply. Never extract from a truncated
            // reply. (Prose braces fail with non-EOF errors and fall through.)
            //
            // Which error is surfaced matters as much as the refusal. A
            // container that fails to parse for a reason OTHER than running
            // out of input is malformed, not truncated — a stray unescaped
            // quote inside a shell command, a trailing comma — and the reply
            // is otherwise complete. Returning `root_error` there tells the
            // repair loop "expected value at line 1 column 1", which is true
            // of the prose preamble and useless as a correction: observed
            // live, an agent re-emitted the same 41k-char envelope with the
            // same bad escape because nothing ever named the real fault.
            // Serde's own error carries the line and column, so hand that
            // back instead and let the repair prompt quote it.
            Some(Err(error)) if error.is_eof() => {
                return Err(root_error);
            }
            Some(Err(error)) if starts_like_json_container(&output[index..]) => {
                return Err(error);
            }
            _ => {}
        }
    }
    last_envelope.ok_or(root_error)
}

/// Distinguish the real result envelope from a nested task_coverage entry when
/// extracting one object from a prose/fence-wrapped reply. Both carry `status`.
/// A top-level `task_id` previously marked the object a fragment and rejected the
/// WHOLE (valid) result — but agents routinely add a top-level `task_id` to the
/// envelope (observed live: verify branches emitting `{status, task_id, …}` got
/// binned as "missing field status"). The envelope always carries orchestration
/// fields a task_coverage entry never has, so key on those: accept when `status`
/// is present and either there is no `task_id` or at least one envelope-only
/// field is present. A bare `{task_id, status, summary, evidence}` fragment is
/// still rejected.
fn is_result_envelope(value: &Value) -> bool {
    if value.get("status").is_none() {
        return false;
    }
    if value.get("task_id").is_none() {
        return true;
    }
    const ENVELOPE_ONLY: [&str; 7] = [
        "commands_run",
        "files_changed",
        "files_read",
        "artifacts",
        "residual_gaps",
        "task_coverage",
        "data",
    ];
    ENVELOPE_ONLY.iter().any(|key| value.get(key).is_some())
}

/// Decide whether a bracket that FAILED to parse was a real JSON container or
/// just punctuation in the agent's prose. A malformed container is fatal: no
/// complete object inside it may be promoted to the reply envelope. Prose is
/// skipped, and scanning continues to the envelope that follows.
///
/// The discriminator is the first non-whitespace character after the opener,
/// because that is where JSON and prose diverge unambiguously.
fn starts_like_json_container(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    match chars.next() {
        // Valid JSON object members begin with a quoted key. If such a
        // container is malformed later, no complete object inside it may be
        // promoted to the reply envelope. Natural-language braces such as
        // "{ curly braces" remain eligible prose.
        Some('{') => matches!(chars.find(|ch| !ch.is_whitespace()), Some('"' | '}')),
        // A JSON array element opens with a quote, a nested container, a
        // number, or the array closes immediately. Treating EVERY `[` as a
        // container instead was the second array exit from this loop, and it
        // outlived the reason for it: once multi-object output resolved to
        // "take the last envelope", a bracket in prose that is not valid JSON
        // became the only remaining way for a complete, valid envelope to be
        // thrown away. Observed live and repeatedly on this workspace — a
        // verification branch wrote "contains 7 `#[test]` annotations" in its
        // preamble, `[test]` failed as an array at column 3, and 40 lines of
        // finished, well-formed result went in the bin. `[dependencies]`,
        // `[cfg(...)]` and every other Rust-flavoured bracket read the same.
        //
        // The shape this arm actually guards against is unaffected: a
        // malformed `[{...}, garbage]` still opens with `{`, is still a
        // container, and its contents are still never promoted.
        Some('[') => matches!(
            chars.find(|ch| !ch.is_whitespace()),
            Some('"' | '{' | '[' | ']' | '-' | '0'..='9')
        ),
        _ => false,
    }
}

fn normalize_path_records(object: &mut Map<String, Value>, field: &str) {
    let Some(records) = object.get_mut(field).and_then(Value::as_array_mut) else {
        return;
    };
    for record in records {
        let Some(path) = record.as_str() else {
            continue;
        };
        *record = serde_json::json!({"path": path});
    }
}

fn stamp_envelope(request: &WorkflowV2AgentRequest, object: &mut Map<String, Value>) {
    insert_missing(object, "id", Value::String(request.call.id.clone()));
    insert_missing(object, "stage", Value::String(request.call.id.clone()));
    insert_missing(
        object,
        "attempt",
        serde_json::json!(request_attempt(request)),
    );
    if let Some(run_id) = &request.project_artifacts.run_id {
        insert_missing(object, "workflow_id", Value::String(run_id.clone()));
    }
}

fn request_attempt(request: &WorkflowV2AgentRequest) -> u64 {
    request
        .input
        .get("attempt")
        .and_then(Value::as_u64)
        .unwrap_or(1)
}

fn stamp_artifact_ids(object: &mut Map<String, Value>) {
    let Some(artifacts) = object.get_mut("artifacts").and_then(Value::as_array_mut) else {
        return;
    };
    for (index, artifact) in artifacts.iter_mut().enumerate() {
        let Some(fields) = artifact.as_object_mut() else {
            continue;
        };
        if fields.get("id").is_some_and(value_present) {
            continue;
        }
        let path = fields
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("artifact");
        fields.insert("id".to_string(), Value::String(artifact_id(index, path)));
    }
}

fn normalize_commands(object: &mut Map<String, Value>) {
    let Some(commands) = object.get_mut("commands_run").and_then(Value::as_array_mut) else {
        return;
    };
    for command in commands {
        let Some(fields) = command.as_object_mut() else {
            continue;
        };
        insert_missing(fields, "kind", Value::String("other".to_string()));
        derive_command_status_from_exit_code(fields);
        normalize_command_status(fields);
        synthesize_missing_output_summary(fields);
    }
}

/// `output_summary` is a required String on the result envelope but is
/// descriptive metadata only: the command's own `status`/`exit_code` drive
/// every gate, and the verification pass/fail detector keys on explicit
/// pass/failure markers. Agents intermittently omit it, which otherwise
/// hard-rejects an entire substantively-valid result (a real defect observed
/// on live runs). Synthesize a NEUTRAL placeholder when absent — it carries no
/// pass or failure marker, so verification stays fail-closed and can never be
/// tricked into reading a pass; the schema and every safety gate still run on
/// the real result. Only fills a missing/empty value; never overwrites what the
/// agent actually reported.
fn synthesize_missing_output_summary(fields: &mut Map<String, Value>) {
    if fields.get("output_summary").is_some_and(value_present) {
        return;
    }
    let status = fields
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("reported");
    fields.insert(
        "output_summary".to_string(),
        Value::String(format!(
            "(no output_summary provided by agent; command status: {status})"
        )),
    );
}

/// Derive a missing command status from the exit code the agent already gave.
///
/// A `commands_run` entry with no `status` fails the WHOLE reply — the field
/// has no default, deliberately, because a wrongly-normalised command status
/// is how a failed test gets recorded as a pass. Observed live: a verification
/// branch recorded two commands, the second carrying `command`, `kind`,
/// `output_summary` and `exit_code` but no `status`, and the entire envelope
/// was rejected with "missing field `status`" after the branch had done all
/// its work.
///
/// The exit code is not a guess — it is the outcome, stated by the agent, in
/// the same record. `0` succeeded, anything else failed. Where there is no
/// exit code there is nothing to infer from and the reply is still rejected,
/// so this cannot invent a pass out of silence.
fn derive_command_status_from_exit_code(fields: &mut Map<String, Value>) {
    if fields.get("status").is_some_and(value_present) {
        return;
    }
    let Some(code) = fields.get("exit_code").and_then(Value::as_i64) else {
        return;
    };
    let derived = if code == 0 { "succeeded" } else { "failed" };
    fields.insert("status".to_string(), Value::String(derived.to_string()));
}

fn normalize_command_status(fields: &mut Map<String, Value>) {
    let Some(status) = fields.get("status").and_then(Value::as_str) else {
        return;
    };
    let canonical = match status.to_ascii_lowercase().as_str() {
        "passed" | "ok" | "success" => "succeeded",
        "failure" | "error" => "failed",
        "skip" => "skipped",
        _ => return,
    };
    fields.insert("status".to_string(), Value::String(canonical.to_string()));
}

fn artifact_id(index: usize, path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or("artifact");
    let safe: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    format!("artifact-{index}-{}", safe.trim_matches('-'))
}

fn insert_missing(object: &mut Map<String, Value>, key: &str, value: Value) {
    if !object.get(key).is_some_and(value_present) {
        object.insert(key.to_string(), value);
    }
}

fn value_present(value: &Value) -> bool {
    !value.is_null() && value.as_str().is_none_or(|value| !value.trim().is_empty())
}
