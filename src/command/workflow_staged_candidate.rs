//! Issue-367: which bytes of a body author's answer are the task file.
//!
//! A task file opens with its ```` ```yaml ```` frontmatter. The answer is the
//! task file when its first non-blank line is that opener; the rest is kept
//! byte for byte (a task file legitimately ends with prose and inner code
//! blocks, so nothing after the frontmatter is inspected here). An answer
//! wrapped whole in one outer fence (Issue-61) is unwrapped. Any other answer
//! has text before the task file, and is refused with one finding the author
//! can act on, instead of landing a file every lint then misreads.

use std::path::Path;

use crate::command::topology_lint::{
    first_nonblank_line, is_frontmatter_opener, strip_leading_blank_lines, unwrap_outer_fence,
};
use crate::command::workflow_gate::{GateEvaluation, GateFinding, GateId};

/// The task file in `candidate`, and whether an outer fence was removed; or
/// the one finding text that refuses it. Bytes that are not UTF-8 pass
/// through untouched for the mechanical checks to report.
pub(super) fn normalize_task_candidate(candidate: Vec<u8>) -> Result<(Vec<u8>, bool), String> {
    let Ok(text) = std::str::from_utf8(&candidate) else {
        return Ok((candidate, false));
    };
    let rest = strip_leading_blank_lines(text);
    if is_frontmatter_opener(first_nonblank_line(rest)) {
        if rest.len() == text.len() {
            return Ok((candidate, false));
        }
        return Ok((rest.as_bytes().to_vec(), false));
    }
    if let Some(interior) = unwrap_outer_fence(rest) {
        let inner = strip_leading_blank_lines(interior);
        if is_frontmatter_opener(first_nonblank_line(inner)) {
            return Ok((inner.as_bytes().to_vec(), true));
        }
        return Err(text_before_task_file(first_nonblank_line(inner)));
    }
    Err(text_before_task_file(first_nonblank_line(rest)))
}

/// The landed-file shape check: a UTF-8 task file about to land opens with
/// its frontmatter; else the finding text that refuses it. Bytes that are not
/// UTF-8 never reach here (the lint refuses them first).
pub(super) fn landed_shape(bytes: &[u8]) -> Result<(), String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(());
    };
    let first = first_nonblank_line(text);
    if is_frontmatter_opener(first) {
        Ok(())
    } else {
        Err(text_before_task_file(first))
    }
}

/// Stage the refusal of a candidate whose shape is not a task file: the gate
/// envelope alone, carrying one `Body` finding, so the host routes it back to
/// the author as a retry (`candidate refused before staging`). The code is
/// the shared shape refusal's, so the script measures it as a refused
/// attempt at the shape stage and any landed body after it as progress.
pub(super) fn stage_shape_refusal(
    staging_root: &Path,
    gate_envelope: &Path,
    call_id: &str,
    path: &Path,
    reason: String,
) -> anyhow::Result<archon_workflow::PreparedPublicationV1> {
    let finding = GateFinding::new(
        GateId::WorkflowLintTaskFile,
        reason,
        format!("task file {}", path.display()),
        Some(path.to_path_buf()),
        archon_workflow::RemediationScope::Body,
    )
    .with_defect(archon_workflow::defect::DeterministicDefect::new(
        "invalid_candidate_shape",
        "task_file",
        "candidate",
    ));
    crate::command::workflow_gate_envelope::stage_gate_evaluation(
        staging_root,
        gate_envelope,
        call_id,
        "land-task-body",
        GateEvaluation::new("candidate task file shape refused", vec![finding]),
        Vec::new(),
    )
}

fn text_before_task_file(first_line: &str) -> String {
    let quoted: String = first_line.trim().chars().take(120).collect();
    format!(
        "the answer has text before the task file (first line: \"{quoted}\"); return only the task file, starting with its ```yaml frontmatter block"
    )
}
