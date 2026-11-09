//! Author-visible policy proofs for #631.
use super::*;
#[path = "../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r7_9_python_repair_example_is_language_neutral() {
    let request:StageRunRequest=serde_json::from_value(serde_json::json!({"run_id":"proof","stage_id":"python-discovery","stage_kind":"agent","agent":null,"task":"Inspect proof.py","attempt":1,"provider_tier":"coder","depends_on":[],"input":{}})).unwrap();
    let prompt = item_output_repair_prompt(&request, "{}");
    assert!(
        !prompt.contains("repository/relative/path.rs"),
        "Python schema repair receives only a Rust example target"
    );
}
