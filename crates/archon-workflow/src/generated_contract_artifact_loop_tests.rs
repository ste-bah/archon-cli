//! Regression tests for the artifact-requirements repair loop that blocked
//! run wf-8550dd8f (and, in earlier shapes, two runs before it).
//!
//! Two independent defects combined into an unwinnable loop:
//!
//! 1. **Trigger** — an artifact declaration of the shape
//!    `{"kind": "...", "min_instances": 0, "paths": ["..."]}` was classified
//!    invalid because the reader knew `path` (singular) but not `paths`.
//!    The declaration carried concrete paths; the reader did not know the key.
//!    The same hole was fixed once before for `absolute_path` — see the
//!    `explicit_path` doc comment — and reopened under a new key.
//!
//! 2. **Lock** — `artifact_requirement_issues` is derived during
//!    normalization, but a stale copy on the incoming value survived
//!    re-normalization because the splitter only ever appended to it. A
//!    repair that replaced `artifact_requirements` with clean paths could
//!    therefore never clear the flag, the validator re-raised the same issue,
//!    the issue count never moved, and `adopt_inventory_repair` rejected five
//!    correct repairs in a row until the loop cap blocked the run.

use super::*;

fn artifact_issue_count(inventory: &NormalizedGeneratedInventory) -> usize {
    inventory
        .issues
        .iter()
        .filter(|issue| issue.kind == GeneratedContractIssueKind::ArtifactRequirementsDiscovery)
        .count()
}

/// The exact declaration shape `inventory-shape-repair-1` emitted live, one
/// per split item: object metadata plus a plural `paths` array.
#[test]
fn object_with_plural_paths_array_is_a_concrete_declaration() {
    let raw = serde_json::json!({
        "items": [{
            "item_id": "ITEM-1",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-1"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["compiles"],
            "focused_verification": ["cargo check"],
            "artifact_requirements": [
                {"kind": "create", "min_instances": 0, "paths": ["src/generated/out.rs"]}
            ]
        }]
    });
    let inventory = normalize_generated_inventory_value(&raw, None);
    assert_eq!(
        artifact_issue_count(&inventory),
        0,
        "a paths-array declaration is concrete, not invalid: {:?}",
        inventory.issues
    );
    let requirements = inventory.items[0]["artifact_requirements"].to_string();
    assert!(
        requirements.contains("src/generated/out.rs"),
        "the declared path must survive normalization: {requirements}"
    );
}

/// A stale `artifact_requirement_issues` marker on the incoming item must not
/// outlive re-normalization when the current declarations are clean. This is
/// what let the validator re-raise a resolved issue forever: the repair fixed
/// `artifact_requirements`, the flag lived in a different field, and nothing
/// ever recomputed it.
#[test]
fn stale_artifact_issue_marker_is_recomputed_not_inherited() {
    let raw = serde_json::json!({
        "items": [{
            "item_id": "ITEM-1",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-1"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["compiles"],
            "focused_verification": ["cargo check"],
            "artifact_requirements": ["docs/report.md"],
            "artifact_requirement_issues": [
                {"kind": "left-over-from-before-the-repair"}
            ]
        }]
    });
    let inventory = normalize_generated_inventory_value(&raw, None);
    assert_eq!(
        artifact_issue_count(&inventory),
        0,
        "clean declarations must clear a stale marker: {:?}",
        inventory.issues
    );
}

/// A genuinely malformed declaration still raises the issue — the marker is
/// recomputed, not suppressed.
#[test]
fn genuinely_invalid_declarations_still_raise_the_issue() {
    let raw = serde_json::json!({
        "items": [{
            "item_id": "ITEM-1",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-1"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["compiles"],
            "focused_verification": ["cargo check"],
            "artifact_requirements": [
                {"kind": "mystery", "min_instances": 1}
            ]
        }]
    });
    let inventory = normalize_generated_inventory_value(&raw, None);
    assert_eq!(
        artifact_issue_count(&inventory),
        1,
        "an object with no path and no evidence is still invalid: {:?}",
        inventory.issues
    );
}

/// An object carrying both a plural paths array and evidence text keeps both:
/// the paths stay concrete declarations and the text stays evidence. Without
/// the ordering fix the evidence branch won and the paths were dropped.
#[test]
fn plural_paths_with_evidence_text_keep_both() {
    let raw = serde_json::json!({
        "items": [{
            "item_id": "ITEM-1",
            "work_type": "implementation",
            "canonical_task_ids": ["TASK-1"],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["compiles"],
            "focused_verification": ["cargo check"],
            "artifact_requirements": [{
                "kind": "report",
                "paths": ["docs/report.md"],
                "description": "report lists every dataset cell"
            }]
        }]
    });
    let inventory = normalize_generated_inventory_value(&raw, None);
    assert_eq!(
        artifact_issue_count(&inventory),
        0,
        "{:?}",
        inventory.issues
    );
    let item = inventory.items[0].to_string();
    assert!(item.contains("docs/report.md"), "paths kept: {item}");
    assert!(
        item.contains("report lists every dataset cell"),
        "evidence kept: {item}"
    );
}
