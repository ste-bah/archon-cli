//! What a write branch is told about its copy of the project's acceptance
//! inputs, and the gaps that say what of it did not land (Batch E; see
//! `project_inputs_seed`).

use crate::v2::{WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status};
use crate::write_coordinator::project_inputs::SeedRecord;

/// Gap id prefix of a branch whose project-input landing was refused.
pub(crate) const PROJECT_INPUT_REFUSED_GAP_PREFIX: &str = "project_inputs_refused_";
/// Gap id prefix of repository test material refused as project data.
pub(crate) const PROJECT_INPUT_FIXTURE_GAP_PREFIX: &str = "project_inputs_test_fixture_";
/// Gap id prefix of a branch whose project-input changes were not all kept.
pub(crate) const PROJECT_INPUT_DROPPED_GAP_PREFIX: &str = "project_inputs_not_landed_";

/// What the agent is told about its copy of the project's data.
pub(super) fn preamble(record: &SeedRecord) -> String {
    if record.inputs.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "\nProject data: the acceptance checks run against the project's own data under {} \
         (relative to the project root). This worktree holds a copy of it at the same relative \
         paths ({} file(s)). Run the product's own commands against it as the checks do. What \
         you change there is applied to the project root when this branch lands; it is never \
         part of your git patch, and a file the project's copy changed after this worktree was \
         seeded is refused and reported, not overwritten. Data you land there must come from \
         the product's own real ingestion paths run against real sources -- never a copy of, \
         or an ingest of, a repository test fixture or a hand-made sample -- unless the task \
         spec explicitly says fixtures are the deliverable: a landing holding a copy of a \
         tracked test file, or naming one as its source, is refused as a HIGH finding, and \
         the verifier judges the provenance of every file you land.",
        record.inputs.join(", "),
        record.files.len()
    );
    if !record.skipped.is_empty() {
        // An exclusion for shadowing the frozen contract is always listed.
        let shadow =
            |why: &str| why.starts_with(super::project_inputs_seed::SHADOWS_FROZEN_CONTRACT);
        let listed: Vec<String> = (record.skipped.iter().filter(|(_, why)| shadow(why)))
            .chain(record.skipped.iter().filter(|(_, why)| !shadow(why)))
            .take(10)
            .map(|(path, why)| format!("{path} ({why})"))
            .collect();
        text.push_str(&format!(
            " Not copied: {}{}.",
            listed.join("; "),
            if record.skipped.len() > 10 {
                format!("; and {} more", record.skipped.len() - 10)
            } else {
                String::new()
            }
        ));
    }
    text.push('\n');
    text
}

/// Say on the branch what of its project data will not land: nothing at
/// all when its capture was refused, or the paths it left out. HIGH: the
/// data may be what its checks read. The patch is unaffected either way.
pub(super) fn report_capture(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    capture: &super::project_inputs_seed::InputCapture,
) {
    let mut lines: Vec<String> = capture
        .dropped
        .iter()
        .take(20)
        .map(|(path, why)| format!("{path} ({why})"))
        .collect();
    if capture.dropped.len() > 20 {
        lines.push(format!("and {} more", capture.dropped.len() - 20));
    }
    let description = match &capture.refused {
        Some(reason) => format!(
            "none of this branch's changes to the project's data will land: {reason}. Its patch is unaffected."
        ),
        None if lines.is_empty() => return,
        None => format!(
            "these changes to the project's data were left out and will not land: {}. The rest land with the branch.",
            lines.join("; ")
        ),
    };
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("{PROJECT_INPUT_DROPPED_GAP_PREFIX}{branch_id}"),
        description,
        severity: Some("high".to_string()),
    });
}

/// A landing whose project-input changes were refused has not delivered what
/// its branch reported: the item is downgraded, as an unapplied patch is,
/// with a HIGH gap naming what was refused and why.
pub(super) fn report_refusals(
    artifacts: &mut super::WorktreeWaveArtifacts,
    refusals: &[(crate::write_coordinator::ItemId, String)],
) {
    for (item_id, reason) in refusals {
        let Some(index) = artifacts
            .completed
            .iter()
            .position(|branch| branch.item_id.as_str() == item_id.as_str())
        else {
            continue;
        };
        let Some(result) = artifacts.results.get_mut(index) else {
            continue;
        };
        result.status = WorkflowV2Status::NeedsReview;
        if let Some(data) = result.data.as_object_mut() {
            data.insert("project_inputs_refused".into(), serde_json::json!(reason));
        }
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("{PROJECT_INPUT_REFUSED_GAP_PREFIX}{item_id}"),
            description: format!(
                "this branch's changes to the project's acceptance inputs were NOT applied to the project root: {reason}. What its patch carried landed; its project data did not."
            ),
            severity: Some("high".to_string()),
        });
    }
}

/// Batch K (I1): each piece of repository test material a landing refused
/// as project data is a HIGH finding on its branch, whatever else it did.
pub(super) fn report_fixtures(
    artifacts: &mut super::WorktreeWaveArtifacts,
    findings: &[(crate::write_coordinator::ItemId, String)],
) {
    for (n, (item_id, finding)) in findings.iter().enumerate() {
        let Some(index) = artifacts
            .completed
            .iter()
            .position(|branch| branch.item_id.as_str() == item_id.as_str())
        else {
            continue;
        };
        let Some(result) = artifacts.results.get_mut(index) else {
            continue;
        };
        result.status = WorkflowV2Status::NeedsReview;
        if let Some(data) = result.data.as_object_mut() {
            let listed = data
                .entry("project_inputs_test_fixtures")
                .or_insert_with(|| serde_json::json!([]));
            if let Some(list) = listed.as_array_mut() {
                list.push(serde_json::json!(finding));
            }
        }
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("{PROJECT_INPUT_FIXTURE_GAP_PREFIX}{item_id}_{n}"),
            description: finding.clone(),
            severity: Some("high".to_string()),
        });
    }
}
