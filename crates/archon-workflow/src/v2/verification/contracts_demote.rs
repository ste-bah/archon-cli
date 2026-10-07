//! A branch demoted for a failed declared contract, with every finding the
//! contract reported (Issue 219): none dropped, all bounded.
//!
//! A generated verifier reports one failure per missing instance, so a
//! failure can carry thousands of findings. Every one is kept, and nothing
//! the branch carries grows with their number:
//!
//! - the complete list is written to an evidence file in the run directory
//!   (`v2/declared-contract-findings/`), whose path the result names;
//! - the residual gap -- the text every remediate and verify prompt quotes
//!   -- and `data.declared_contract_findings` hold findings up to
//!   [`GAP_BUDGET_BYTES`], each cut at [`FINDING_BYTES`] with a mark, and
//!   then "N more finding(s): full list at <path>", so the agent that fixes
//!   them can read the rest;
//! - with no run directory (or a file that could not be written), the data
//!   keeps every finding, each bounded, and the gap names it.

use std::path::{Path, PathBuf};

use super::*;

/// Most bytes one finding contributes to the gap or the data.
pub(in crate::v2::verification) const FINDING_BYTES: usize = 4096;
/// Most bytes of findings the gap and the data quote before pointing at the
/// evidence file.
pub(in crate::v2::verification) const GAP_BUDGET_BYTES: usize = 16 * 1024;
/// Where the complete lists are written, under the run directory.
pub(in crate::v2::verification) const FINDINGS_DIR: &str = "v2/declared-contract-findings";

/// `text` within `max` bytes, cut at a character boundary with a mark.
fn bounded(text: &str, max: usize, whole: &str) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{} [finding cut at {end} of {} bytes; whole {whole}]",
        &text[..end],
        text.len()
    )
}

/// The complete list, written whole; its path, or why it was not written.
fn write_evidence(
    run_root: Option<&Path>,
    item_id: &str,
    findings: &[String],
) -> Result<PathBuf, String> {
    let root = run_root.ok_or("no run directory")?;
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "item_id": item_id,
        "finding_count": findings.len(),
        "findings": findings,
    }))
    .map_err(|error| error.to_string())?;
    let slug: String = (item_id.chars())
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(60)
        .collect();
    let digest = blake3::hash(&bytes).to_hex();
    let dir = root.join(FINDINGS_DIR);
    let path = dir.join(format!("{slug}-{}.json", &digest[..16]));
    let temporary = path.with_extension("json.tmp");
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&temporary, &bytes))
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

/// The findings the gap and data quote, and the gap's text.
fn quoted(findings: &[String], evidence: &Result<PathBuf, String>) -> (Vec<String>, String) {
    let whole = match evidence {
        Ok(path) => format!("at {}", path.display()),
        Err(_) => "in data.declared_contract_findings".to_string(),
    };
    let mut shown = Vec::new();
    let mut used = 0usize;
    for finding in findings {
        let entry = bounded(finding, FINDING_BYTES, &whole);
        // With no file, every finding stays (each bounded): none is dropped.
        if evidence.is_ok() && !shown.is_empty() && used + entry.len() > GAP_BUDGET_BYTES {
            break;
        }
        used += entry.len() + 2;
        shown.push(entry);
    }
    let mut text = format!("{} finding(s)", findings.len());
    match evidence {
        Ok(path) => text.push_str(&format!(" (full list at {})", path.display())),
        Err(why) => text.push_str(&format!(
            " (full list in data.declared_contract_findings; no evidence file: {why})"
        )),
    }
    text.push_str(": ");
    text.push_str(&shown.join("; "));
    if let (Ok(path), true) = (evidence, shown.len() < findings.len()) {
        text.push_str(&format!(
            "; {} more finding(s): full list at {}",
            findings.len() - shown.len(),
            path.display()
        ));
    }
    (shown, text)
}

/// Demote `outcome` for a failed declared contract carrying `findings`.
pub(crate) fn demote_failed_contract(
    outcome: &mut WorkflowV2BranchOutcome,
    findings: &[String],
    run_root: Option<&Path>,
) {
    let evidence = write_evidence(run_root, &outcome.item_id, findings);
    let (shown, text) = quoted(findings, &evidence);
    if let Some(result) = outcome.result.as_mut() {
        result.status = WorkflowV2Status::NeedsReview;
        result.residual_gaps.push(crate::WorkflowV2ResidualGap {
            id: "declared_contract_verification_failed".to_string(),
            description: format!(
                "host-executed declared deliverable contract verification failed: {text}"
            ),
            severity: Some("review".to_string()),
        });
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            "accepted branch demoted: the host ran the declared deliverable contract verifier and it failed",
        ));
        let mut data = result.data.as_object().cloned().unwrap_or_default();
        data.insert("declared_contract_verification".into(), "failed".into());
        data.insert(
            "declared_contract_findings".into(),
            serde_json::json!(shown),
        );
        data.insert(
            "declared_contract_finding_count".into(),
            serde_json::json!(findings.len()),
        );
        if let Ok(path) = &evidence {
            data.insert(
                "declared_contract_findings_path".into(),
                serde_json::json!(path.display().to_string()),
            );
        }
        data.insert(
            "verification_failure_class".into(),
            "declared_contract_violation".into(),
        );
        result.data = serde_json::Value::Object(data);
    }
    outcome.status = WorkflowV2Status::NeedsReview;
    outcome.failure_kind = Some(BranchFailureKind::Semantic);
}

/// Separate diagnostics accompany the existing demotion, never a finding.
pub(super) fn attach_environment_note(outcome: &mut WorkflowV2BranchOutcome, note: &str) {
    if let Some(result) = &mut outcome.result {
        result.data["check_environment_note"] = serde_json::json!(note);
        if let Some(gap) = result.residual_gaps.last_mut() {
            gap.description.push_str(&format!("\n{note}"));
        }
    } else {
        let error = outcome.error.get_or_insert_with(String::new);
        if !error.is_empty() {
            error.push('\n');
        }
        error.push_str(note);
    }
}
