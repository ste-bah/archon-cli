//! What a `verified_noop` inventory item must prove.
//!
//! Split from `generated_contract_validation.rs` for the 500-line ceiling. The
//! caller is the `work_type` match in `generated_item_issues`.

use super::*;

/// The rules a no-op claim must satisfy before it can close a task.
pub(super) fn verified_noop_issues(
    value: &serde_json::Value,
    universe: &ContractTaskUniverse,
    canonical_task_ids: &[String],
    issues: &mut Vec<GeneratedContractIssue>,
    make_issue: &dyn Fn(GeneratedContractIssueKind, &str, &str) -> GeneratedContractIssue,
) {
    // A task whose deliverable contract names a command to RUN cannot be
    // finished by inspection. The harness invites every implementation
    // agent to answer `idempotent_noop` when "the implementation is
    // already complete and no repository change is required", and for an
    // execution task that invitation is a trap: the artifacts exist
    // because someone wrote the source, and pointing at them satisfies
    // the no-op proof rule while the command never runs.
    //
    // Observed across four runs of one PRD: ingest tasks accepted with
    // changed=0 and a registry that stayed empty, then refuted by
    // verification, then re-run and accepted as no-ops again.
    //
    // Executed evidence means `commands_run` — the record of something
    // actually happening. Artifacts and proof references describe what is
    // on disk, which is exactly the claim in doubt.
    if universe.requires_execution(canonical_task_ids) && !value_present(value.get("commands_run"))
    {
        issues.push(make_issue(
            GeneratedContractIssueKind::EvidenceRepair,
            "commands_run",
            "verified_noop is not available for a task whose deliverable \
                 contract declares a command to execute: record the command in \
                 commands_run, or classify this as implementation work",
        ));
    }
    // A no-op that verification already refuted cannot be re-proposed as
    // a no-op. `noop_routing::implementation_item` stamps
    // `noop_reclassification` onto exactly the items whose no-op claim
    // was tested and failed, carrying the refuted claim and the gaps
    // that refuted it — so the marker is present only on a second pass
    // over ground already lost.
    //
    // Observed across runs of one PRD: an item accepted as a no-op, the
    // verification wave refuting it, and the retry returning the same
    // no-op with the same proof. The refutation was in the retry's own
    // prompt as `required_fix` and `noop_reclassification`; the agent
    // read it and answered the same way regardless, because nothing
    // stopped it. Prompt text is a request; this is the rule.
    if value_present(value.get("noop_reclassification")) {
        issues.push(make_issue(
            GeneratedContractIssueKind::EvidenceRepair,
            "work_type",
            "this item's no-op claim was already refuted by verification: \
                 it cannot be re-classified verified_noop, so classify it as \
                 implementation work and satisfy the refuted criteria",
        ));
    }
    if !value_present(value.get("acceptance_criteria")) {
        issues.push(make_issue(
            GeneratedContractIssueKind::VerificationRequirementsDiscovery,
            "acceptance_criteria",
            "verified_noop item is missing acceptance criteria",
        ));
    }
    if !value_present(value.get("noop_proof")) || !value_present(value.get("noop_proof_refs")) {
        issues.push(make_issue(
            GeneratedContractIssueKind::EvidenceRepair,
            "noop_proof_refs",
            "verified_noop item is missing no-op proof or proof references",
        ));
    }
    if value.get("artifact_requirements").is_none() {
        issues.push(make_issue(
            GeneratedContractIssueKind::ArtifactRequirementsDiscovery,
            "artifact_requirements",
            "verified_noop item is missing artifact requirements metadata",
        ));
    }
}
