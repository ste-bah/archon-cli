//! Issue-367: which bytes of a body author's answer are the task file.
//!
//! A task file opens with its ```` ```yaml ```` frontmatter. The answer is the
//! task file when its first non-blank line is that opener; the rest is kept
//! byte for byte (a task file legitimately ends with prose and inner code
//! blocks, so nothing after the frontmatter is inspected here). An answer
//! wrapped whole in one outer fence (Issue-61) is unwrapped. Text before the
//! task file (chat, optionally with an outer wrapper) is packaging the host
//! discards exactly ([`strip_packaging`]) and records in the report. Any
//! other answer is refused with one finding, instead of landing a file every
//! lint then misreads.

use std::path::Path;

use crate::command::topology_lint::{
    first_nonblank_line, is_frontmatter_opener, strip_leading_blank_lines, strip_packaging,
    unwrap_outer_fence,
};

/// The task file the host lands from a body author's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TaskCandidate {
    /// The task file's bytes, exactly as written inside any packaging.
    pub(super) bytes: Vec<u8>,
    /// Whether an outer wrapper fence pair was removed.
    pub(super) unwrapped: bool,
    /// What the host discarded before the task file, when it discarded any.
    pub(super) packaging: Option<Packaging>,
}

/// The packaging the host discarded before a task file: a diagnostic for the
/// envelope's report, never a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Packaging {
    pub(super) lines: usize,
    pub(super) bytes: usize,
    pub(super) wrapper: bool,
    /// The first 200 characters of the discarded text, redacted.
    pub(super) preview: String,
}

impl Packaging {
    fn of(leading: &str, wrapper: bool) -> Self {
        // Redact first, so a cut never leaves part of a secret-shaped word.
        let redacted = match archon_workflow::events::sanitize_value(serde_json::Value::String(
            leading.to_string(),
        )) {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        };
        Self {
            lines: leading.lines().count(),
            bytes: leading.len(),
            wrapper,
            preview: redacted.chars().take(200).collect(),
        }
    }

    /// The envelope report's note: what was discarded, so nothing the host
    /// removed is silent.
    pub(super) fn report(&self) -> String {
        format!(
            "\n## candidate normalisation\n  the host discarded packaging before the task file: {} leading line(s), {} byte(s); outer wrapper fence pair removed: {}\n  discarded text (first 200 characters, redacted): {:?}\n",
            self.lines,
            self.bytes,
            if self.wrapper { "yes" } else { "no" },
            self.preview
        )
    }
}

impl TaskCandidate {
    fn plain(bytes: Vec<u8>, unwrapped: bool) -> Self {
        Self {
            bytes,
            unwrapped,
            packaging: None,
        }
    }
}

/// The task file in `candidate`; or the one finding text that refuses it.
/// Bytes that are not UTF-8 pass through untouched for the mechanical checks
/// to report.
pub(super) fn normalize_task_candidate(candidate: Vec<u8>) -> Result<TaskCandidate, String> {
    let Ok(text) = std::str::from_utf8(&candidate) else {
        return Ok(TaskCandidate::plain(candidate, false));
    };
    let rest = strip_leading_blank_lines(text);
    if is_frontmatter_opener(first_nonblank_line(rest)) {
        if rest.len() == text.len() {
            return Ok(TaskCandidate::plain(candidate, false));
        }
        return Ok(TaskCandidate::plain(rest.as_bytes().to_vec(), false));
    }
    if let Some(interior) = unwrap_outer_fence(rest) {
        let inner = strip_leading_blank_lines(interior);
        if is_frontmatter_opener(first_nonblank_line(inner)) {
            return Ok(TaskCandidate::plain(inner.as_bytes().to_vec(), true));
        }
        return Err(text_before_task_file(first_nonblank_line(inner)));
    }
    // Issue-367 follow-up: chat before the task file is packaging, removed
    // here exactly and recorded, because an author that repeats it every
    // attempt can never act on a refusal.
    let Some(packaged) = strip_packaging(text) else {
        return Err(text_before_task_file(first_nonblank_line(rest)));
    };
    let packaging = Packaging::of(packaged.leading, packaged.wrapper);
    tracing::info!(
        lines = packaging.lines,
        bytes = packaging.bytes,
        wrapper = packaging.wrapper,
        "land-task-body discarded packaging before the task file"
    );
    Ok(TaskCandidate {
        bytes: packaged.task_file.as_bytes().to_vec(),
        unwrapped: packaged.wrapper,
        packaging: Some(packaging),
    })
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
    crate::command::workflow_gate_envelope::stage_gate_evaluation(
        staging_root,
        gate_envelope,
        call_id,
        "land-task-body",
        crate::command::workflow_host_command_decision::shape_refusal_evaluation(path, reason),
        Vec::new(),
    )
}

fn text_before_task_file(first_line: &str) -> String {
    let quoted: String = first_line.trim().chars().take(120).collect();
    format!(
        "the answer has text before the task file (first line: \"{quoted}\"); return only the task file, starting with its ```yaml frontmatter block"
    )
}
