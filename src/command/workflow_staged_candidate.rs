//! Issue-367: which bytes of a body author's answer are the task file.
//!
//! A task file opens with its ```` ```yaml ```` frontmatter. The answer is the
//! task file when its first non-blank line is that opener; the rest is kept
//! byte for byte (a task file legitimately ends with prose and inner code
//! blocks, so nothing after the frontmatter is inspected here). An answer
//! wrapped whole in one outer fence (Issue-61) is unwrapped. Text before the
//! task file (chat, optionally with an outer wrapper) is packaging the host
//! discards exactly ([`strip_packaging`]). Every removed byte, a BOM and
//! blank lines included, is recorded in the report. An answer that holds a
//! second task file, or any other shape, is refused with one finding,
//! instead of landing a file every lint then misreads.

use std::path::Path;

use crate::command::topology_lint::{
    first_nonblank_line, is_frontmatter_opener, offset_in, strip_leading_blank_lines,
    strip_packaging, task_files, unwrap_outer_fence,
};

/// The task file the host lands from a body author's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TaskCandidate {
    /// The task file's bytes, exactly as written inside any packaging.
    pub(super) bytes: Vec<u8>,
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

impl TaskCandidate {
    /// Whether an outer wrapper fence pair was removed.
    #[cfg(test)]
    pub(super) fn unwrapped(&self) -> bool {
        self.packaging
            .as_ref()
            .is_some_and(|packaging| packaging.wrapper)
    }
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
            lines: leading.matches('\n').count(),
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

/// The task file in `candidate`; or the one finding text that refuses it.
/// Bytes that are not UTF-8 pass through untouched for the mechanical checks
/// to report. Whatever the host removes is recorded as [`Packaging`], and
/// the rest must hold one task file whichever way it was found.
pub(super) fn normalize_task_candidate(candidate: Vec<u8>) -> Result<TaskCandidate, String> {
    let Ok(text) = std::str::from_utf8(&candidate) else {
        return Ok(TaskCandidate {
            bytes: candidate,
            packaging: None,
        });
    };
    let (task_file, wrapper) = located_task_file(text)?;
    let (first, others) = task_files(task_file);
    if !others.is_empty() {
        return Err(several_task_files(first, &others));
    }
    let leading = &text[..offset_in(text, task_file)];
    let packaging = (!leading.is_empty()).then(|| Packaging::of(leading, wrapper));
    if let Some(packaging) = &packaging {
        tracing::info!(
            lines = packaging.lines,
            bytes = packaging.bytes,
            wrapper = packaging.wrapper,
            "land-task-body discarded packaging before the task file"
        );
    }
    Ok(TaskCandidate {
        bytes: task_file.as_bytes().to_vec(),
        packaging,
    })
}

/// The task file inside `text` and whether a wrapper pair was removed: the
/// answer itself when it opens with the frontmatter (after a BOM and blank
/// lines), the inside of one outer fence (Issue-61), or what follows chat
/// before it ([`strip_packaging`]); else the refusal.
fn located_task_file(text: &str) -> Result<(&str, bool), String> {
    let rest = strip_leading_blank_lines(text);
    if is_frontmatter_opener(first_nonblank_line(rest)) {
        return Ok((rest, false));
    }
    if let Some(interior) = unwrap_outer_fence(rest) {
        let inner = strip_leading_blank_lines(interior);
        if is_frontmatter_opener(first_nonblank_line(inner)) {
            return Ok((inner, true));
        }
        return Err(text_before_task_file(first_nonblank_line(inner)));
    }
    // Issue-367 follow-up: chat before the task file is packaging, removed
    // exactly and recorded, because an author that repeats it every attempt
    // can never act on a refusal.
    match strip_packaging(text) {
        Some(packaged) => Ok((packaged.task_file, packaged.wrapper)),
        None => Err(text_before_task_file(first_nonblank_line(rest))),
    }
}

fn several_task_files(first: Option<String>, others: &[String]) -> String {
    let ids: Vec<&str> = std::iter::once(first.as_deref().unwrap_or("one without a task_id"))
        .chain(others.iter().map(String::as_str))
        .collect();
    format!(
        "the answer holds {} task files ({}); return only one task file, starting with its ```yaml frontmatter block",
        ids.len(),
        ids.join(", ")
    )
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
