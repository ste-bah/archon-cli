#[allow(dead_code)]
#[allow(clippy::duplicate_mod)]
#[path = "workflow_live_v2_lifecycle_prompts.rs"]
mod prompts;

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
