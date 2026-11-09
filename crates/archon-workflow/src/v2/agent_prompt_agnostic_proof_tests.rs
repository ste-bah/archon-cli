//! Issue #631: assertions describe the correct language-neutral behavior.
use super::*;
#[path = "../../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r6_p1_go_verifier_prompt_omits_cargo_policy() {
    let request:WorkflowV2AgentRequest=serde_json::from_value(serde_json::json!({"call":{"id":"verification-plan-go","method":"reduce"},"role":"reducer","task":crate::v2::lifecycle_prompts::VERIFICATION_PLAN_TASK,"input":{"repository_root":fixture::fixture("go"),"declared_focused_tests":["go test . -run TestPassing"]}})).unwrap();
    let prompt = crate::v2::WorkflowV2AgentAdapter::new().build_prompt(&request);
    assert!(
        !prompt.contains("cargo metadata"),
        "Go verifier told to derive package using Cargo"
    );
}

#[test]
fn r6_p2_python_prompt_only_promises_granted_scope() {
    let request:WorkflowV2AgentRequest=serde_json::from_value(serde_json::json!({"call":{"id":"implement-python","method":"implementation","write_mode":"serial"},"role":"coder","task":"Split proof.py at its line cap","repository_root":fixture::fixture("python"),"target_files":["proof.py"],"target_ownership_scopes":[]})).unwrap();
    let prompt = crate::v2::WorkflowV2AgentAdapter::new().build_prompt(&request);
    assert!(
        !prompt.contains("a/b.rs"),
        "Python task receives a Rust-only module-directory ownership promise"
    );
}
