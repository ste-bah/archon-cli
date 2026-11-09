//! Author-visible policy proofs for #631.
use super::*;
#[path = "../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r3_p1_go_frozen_check_describes_own_toolchain() {
    let execution:AcceptanceExecutionRecordV1=serde_json::from_value(serde_json::json!({"mode":"scratch","repository":fixture::fixture("go"),"project":".","task_root":"tasks","environment":"PATH only","timeout_secs":10,"config_present":true})).unwrap();
    let criterion:AcceptanceCriterion=serde_json::from_value(serde_json::json!({"id":"AC-GO-001","criterion":"Addition works","check":{"kind":"command","command":"go test . -run TestPassing","cwd":"repo_root"},"gap_permitted":false,"covers":[],"judgment":{"verdict":"accepted","counterexample":"none","reason":"proof","host_call_id":"judge"}})).unwrap();
    let prompt = frozen_check(
        Path::new("acceptance-contract.json"),
        "digest",
        &execution,
        &criterion,
    );
    assert!(
        !prompt.contains("CARGO_HOME and CARGO_TARGET_DIR"),
        "Go repair prompt describes only Cargo toolchain storage"
    );
}
