use archon_workflow::v2::lifecycle_prompts as prompts;

#[test]
fn workflow_live_generated_semantics_verifier_prompts_forbid_vague_artifact_fields() {
    for prompt in [
        prompts::VERIFICATION_PLAN_TASK,
        prompts::VERIFICATION_PLAN_REPAIR_TASK,
        prompts::VERIFICATION_REPAIR_SHAPE_REPAIR_TASK,
    ] {
        assert!(prompt.contains("enumerate exact"));
        assert!(prompt.contains("all required artifact path fields"));
        assert!(prompt.contains("all expected artifacts"));
    }
}
