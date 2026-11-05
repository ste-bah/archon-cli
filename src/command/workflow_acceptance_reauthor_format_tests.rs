use super::*;

#[test]
fn entry_examples_use_authored_ids_without_a_prefix_rule() {
    let entries: serde_json::Value = serde_json::from_str(&ENTRY_SHAPES).unwrap();
    for entry in entries.as_array().unwrap() {
        assert_eq!(
            entry["covers"][0],
            "<exact requirement id from the PRD whose violation makes this check fail>"
        );
    }
}
