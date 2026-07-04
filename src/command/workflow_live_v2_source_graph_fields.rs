fn remediation_item_has_required_fields(value: &serde_json::Value) -> bool {
    value_present(
        value
            .get("source_item_id")
            .or_else(|| value.get("sourceItemId")),
    ) && value_present(
        value
            .get("failure_status")
            .or_else(|| value.get("failureStatus")),
    ) && value_present(
        value
            .get("failure_evidence")
            .or_else(|| value.get("failureEvidence")),
    ) && value_present(
        value
            .get("required_fix")
            .or_else(|| value.get("requiredFix")),
    ) && value_present(
        value
            .get("verification_requirements")
            .or_else(|| value.get("verificationRequirements")),
    ) && value
        .get("target_files")
        .or_else(|| value.get("targetFiles"))
        .is_some()
        && value
            .get("dependency_ids")
            .or_else(|| value.get("dependencyIds"))
            .or_else(|| value.get("depends_on"))
            .or_else(|| value.get("dependsOn"))
            .is_some()
        && value_present(
            value
                .get("focused_verification")
                .or_else(|| value.get("focusedVerification"))
                .or_else(|| value.get("focused_tests"))
                .or_else(|| value.get("focusedTests")),
        )
        && value
            .get("artifact_requirements")
            .or_else(|| value.get("artifactRequirements"))
            .or_else(|| value.get("artifacts"))
            .is_some()
}

fn review_remediation_item_has_required_fields(value: &serde_json::Value) -> bool {
    value_present(
        value
            .get("source_item_id")
            .or_else(|| value.get("sourceItemId")),
    ) && value_present(
        value
            .get("failure_status")
            .or_else(|| value.get("failureStatus")),
    ) && value_present(
        value
            .get("failure_evidence")
            .or_else(|| value.get("failureEvidence")),
    ) && value_present(
        value
            .get("required_fix")
            .or_else(|| value.get("requiredFix")),
    ) && value
        .get("target_files")
        .or_else(|| value.get("targetFiles"))
        .is_some()
        && value
            .get("dependency_ids")
            .or_else(|| value.get("dependencyIds"))
            .or_else(|| value.get("depends_on"))
            .or_else(|| value.get("dependsOn"))
            .is_some()
        && value_present(
            value
                .get("focused_verification")
                .or_else(|| value.get("focusedVerification"))
                .or_else(|| value.get("focused_tests"))
                .or_else(|| value.get("focusedTests"))
                .or_else(|| value.get("verification_requirements"))
                .or_else(|| value.get("verificationRequirements")),
        )
        && value
            .get("artifact_requirements")
            .or_else(|| value.get("artifactRequirements"))
            .or_else(|| value.get("artifacts"))
            .is_some()
}

fn noop_item_has_required_fields(value: &serde_json::Value) -> bool {
    value_present(value.get("noop_proof").or_else(|| value.get("noopProof")))
        && value_present(
            value
                .get("noop_proof_refs")
                .or_else(|| value.get("noopProofRefs"))
                .or_else(|| value.get("proof_refs"))
                .or_else(|| value.get("proofRefs")),
        )
        && value_present(
            value
                .get("acceptance_criteria")
                .or_else(|| value.get("acceptanceCriteria")),
        )
}

fn verification_item_has_required_fields(value: &serde_json::Value) -> bool {
    value_present(
        value
            .get("focused_verification")
            .or_else(|| value.get("focusedVerification"))
            .or_else(|| value.get("focused_tests"))
            .or_else(|| value.get("focusedTests"))
            .or_else(|| value.get("verification_requirements"))
            .or_else(|| value.get("verificationRequirements")),
    ) && verification_evidence_fields_present(value)
}

fn review_verification_item_has_required_fields(value: &serde_json::Value) -> bool {
    value_present(
        value
            .get("focused_verification")
            .or_else(|| value.get("focusedVerification"))
            .or_else(|| value.get("focused_tests"))
            .or_else(|| value.get("focusedTests"))
            .or_else(|| value.get("verification_requirements"))
            .or_else(|| value.get("verificationRequirements")),
    ) && verification_evidence_fields_present(value)
}

fn verification_evidence_fields_present(value: &serde_json::Value) -> bool {
    value
        .get("artifact_requirements")
        .or_else(|| value.get("artifactRequirements"))
        .or_else(|| value.get("artifacts"))
        .is_some()
        || value
            .get("expected_evidence")
            .or_else(|| value.get("expectedEvidence"))
            .is_some()
}
