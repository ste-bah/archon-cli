//! The patch gate's non-blocking complexity notes, recorded on the branch
//! result.
//!
//! When the complexity cap cannot read a changed file reliably (a syntax
//! error, or the hand scanner losing sync) it does not judge that reading,
//! so a scanner gap never refuses an agent's patch. The note is what keeps
//! that visible: it is written onto the branch result — the persisted call
//! record an operator reads — under `data.complexity_scan_unreliable`, with
//! one evidence line. It is deliberately not a residual gap: nothing about
//! it is the branch's to resolve.
//!
//! The host owns that key: whatever an agent's envelope put there is
//! replaced, or removed when there are no notes, so a planted value can
//! never pose as the host's record.

use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::write_coordinator::patch_manifest::{COMPLEXITY_SCAN_UNRELIABLE, UnreliableScan};

/// Most notes named in the evidence line; all are kept in `data`.
const MAX_NAMED: usize = 5;

pub(super) fn report(result: &mut WorkflowV2Result, notes: &[UnreliableScan]) {
    if notes.is_empty() {
        if let Some(data) = result.data.as_object_mut() {
            data.remove(COMPLEXITY_SCAN_UNRELIABLE);
        }
        return;
    }
    let named: Vec<String> = notes
        .iter()
        .take(MAX_NAMED)
        .map(|note| {
            format!(
                "{}:{} ({}): {}",
                note.path, note.line, note.language, note.reason
            )
        })
        .collect();
    let more = notes.len().saturating_sub(MAX_NAMED);
    let tail = if more > 0 {
        format!("; and {more} more")
    } else {
        String::new()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!(
            "host note ({COMPLEXITY_SCAN_UNRELIABLE}): the complexity cap could not read {} \
             place(s) reliably and did not judge them — a scanner limitation, not a defect \
             in this patch: {}{tail}",
            notes.len(),
            named.join("; ")
        ),
    ));
    if !result.data.is_object() {
        // Kept, not dropped: the host record needs an object to live in.
        let previous = std::mem::take(&mut result.data);
        result.data = if previous.is_null() {
            serde_json::json!({})
        } else {
            serde_json::json!({ "agent_data": previous })
        };
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            COMPLEXITY_SCAN_UNRELIABLE.to_string(),
            serde_json::json!(notes),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(line: usize) -> UnreliableScan {
        UnreliableScan {
            rule: COMPLEXITY_SCAN_UNRELIABLE.to_string(),
            path: "src/a.rs".to_string(),
            line,
            language: "rust".to_string(),
            reason: "post-patch text: syntax error inside function 'f'".to_string(),
        }
    }

    #[test]
    fn notes_are_recorded_on_the_branch_result_without_a_gap() {
        let mut result = WorkflowV2Result::accepted("done");
        report(&mut result, &[note(3), note(9)]);
        let recorded = &result.data[COMPLEXITY_SCAN_UNRELIABLE];
        assert_eq!(recorded.as_array().map(Vec::len), Some(2), "{recorded}");
        assert_eq!(recorded[0]["path"], "src/a.rs");
        assert_eq!(recorded[1]["line"], 9);
        assert_eq!(recorded[0]["language"], "rust");
        assert_eq!(recorded[0]["rule"], COMPLEXITY_SCAN_UNRELIABLE);
        let evidence = &result.evidence.last().expect("evidence").summary;
        assert!(evidence.contains("src/a.rs:3 (rust)"), "{evidence}");
        assert!(result.residual_gaps.is_empty());
        assert_eq!(result.status, crate::v2::result::WorkflowV2Status::Accepted);
    }

    #[test]
    fn notes_replace_a_non_object_data_value_instead_of_being_dropped() {
        let mut result = WorkflowV2Result::accepted("done");
        result.data = serde_json::json!("agent text");
        report(&mut result, &[note(3)]);
        assert_eq!(result.data[COMPLEXITY_SCAN_UNRELIABLE][0]["line"], 3);
        assert_eq!(result.data["agent_data"], "agent text");
    }

    #[test]
    fn the_host_owns_the_key_even_without_notes() {
        let mut result = WorkflowV2Result::accepted("done");
        result.data = serde_json::json!({ COMPLEXITY_SCAN_UNRELIABLE: ["planted"], "kept": 1 });
        report(&mut result, &[]);
        assert!(
            result.data.get(COMPLEXITY_SCAN_UNRELIABLE).is_none(),
            "{}",
            result.data
        );
        assert_eq!(result.data["kept"], 1);
        result.data = serde_json::json!({ COMPLEXITY_SCAN_UNRELIABLE: ["planted"] });
        report(&mut result, &[note(4)]);
        assert_eq!(result.data[COMPLEXITY_SCAN_UNRELIABLE][0]["line"], 4);
    }

    #[test]
    fn no_notes_leave_the_result_untouched() {
        let mut result = WorkflowV2Result::accepted("done");
        let before = serde_json::to_value(&result).expect("json");
        report(&mut result, &[]);
        assert_eq!(serde_json::to_value(&result).expect("json"), before);
    }
}
