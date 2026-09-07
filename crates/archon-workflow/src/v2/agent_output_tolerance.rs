//! Deterministic tolerance for the reply defects that cost live stages.
//!
//! Every repair here has exactly one reading. A closer written where a
//! different closer was due (`]` while an object is still open) is completed
//! with the closer the document itself demands; a residual gap without an id
//! gets one minted from its own description; an artifact declared without a
//! path is credited as evidence of a claim rather than sinking the reply. A
//! reply that simply stops mid-value is not repaired: content may be missing,
//! and the envelope parser keeps failing loudly for it.
use serde_json::{Map, Value};

/// Insert the closer a mismatched closer implies. Returns `None` when nothing
/// was inserted, when a closer matches nothing open, or when the reply ends
/// with containers still open (truncation, which must stay a loud failure).
pub(super) fn repair_mismatched_closers(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len() + 8);
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut inserted = false;
    for ch in input.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if !stack.contains(&ch) {
                    return None;
                }
                while stack.last() != Some(&ch) {
                    out.push(stack.pop()?);
                    inserted = true;
                }
                stack.pop();
            }
            _ => {}
        }
        out.push(ch);
    }
    if in_string || !stack.is_empty() || !inserted {
        return None;
    }
    Some(out)
}

/// A residual gap the agent wrote without an id still names a real gap; mint
/// the id from its description so one omitted field cannot sink the reply.
pub(super) fn stamp_residual_gap_ids(object: &mut Map<String, Value>) {
    let Some(gaps) = object
        .get_mut("residual_gaps")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for (index, gap) in gaps.iter_mut().enumerate() {
        let Some(fields) = gap.as_object_mut() else {
            continue;
        };
        if fields
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
        {
            continue;
        }
        let description = fields
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("gap");
        fields.insert(
            "id".to_string(),
            Value::String(format!("gap-{}-{}", index + 1, slug(description))),
        );
    }
}

/// An artifact declared without a path is a claim the host cannot check. It
/// leaves the artifact list, which is evidence, and is kept as a note, so the
/// claim is visible to review without failing the whole reply.
pub(super) fn credit_pathless_artifacts_as_evidence(object: &mut Map<String, Value>) {
    let Some(artifacts) = object.get_mut("artifacts").and_then(Value::as_array_mut) else {
        return;
    };
    let mut notes = Vec::new();
    artifacts.retain(|artifact| {
        let Some(fields) = artifact.as_object() else {
            return true;
        };
        let has_path = fields
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| !path.trim().is_empty());
        if !has_path {
            let label = fields
                .get("description")
                .or_else(|| fields.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("unnamed");
            notes.push(serde_json::json!({
                "kind": "other",
                "summary": format!("artifact declared without a path was not credited: {label}"),
            }));
        }
        has_path
    });
    if notes.is_empty() {
        return;
    }
    // Beside the agent's own evidence only: the host must not be the one to
    // satisfy the evidence gate.
    if let Some(entries) = object.get_mut("evidence").and_then(Value::as_array_mut)
        && !entries.is_empty()
    {
        entries.extend(notes);
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in text.chars().take(60) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "gap".to_string()
    } else {
        out
    }
}

/// A reply that ends at a value boundary with containers still open owes only
/// its closers: the model wrote `...}]` and stopped one `}` short. Append what
/// the stack still holds. A reply that ends inside a string or mid-literal is
/// a real truncation and is left alone (TD-059).
pub(super) fn complete_missing_closers(input: &str) -> Option<String> {
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in input.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.pop() != Some(ch) {
                    return None;
                }
            }
            _ => {}
        }
    }
    if in_string || stack.is_empty() {
        return None;
    }
    let trimmed = input.trim_end();
    if !trimmed.ends_with(['"', '}', ']']) {
        return None;
    }
    let mut out = trimmed.to_string();
    while let Some(closer) = stack.pop() {
        out.push(closer);
    }
    Some(out)
}

/// `\'` is not a JSON escape; the one thing it can mean is a plain `'`.
/// Rewritten inside strings only, leaving a real `\\` before a quote intact.
pub(super) fn unescape_single_quotes(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut changed = false;
    for ch in input.chars() {
        if in_string {
            if escaped {
                escaped = false;
                if ch == '\'' {
                    out.pop();
                    changed = true;
                }
                out.push(ch);
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            out.push(ch);
            continue;
        }
        if ch == '"' {
            in_string = true;
        }
        out.push(ch);
    }
    changed.then_some(out)
}

#[cfg(test)]
#[path = "agent_output_tolerance_tests.rs"]
mod tests;
