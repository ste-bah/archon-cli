mod branch_count_tests {
    use crate::web_api::branch_count_for_call;
    use std::fs;

    #[test]
    fn branch_count_includes_legacy_items_without_counting_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let store = crate::WorkflowV2ResultStore::new(root);
        let call_id = "call/a";
        let outcome = |item_id: &str| crate::WorkflowV2BranchOutcome {
            item_id: item_id.into(),
            role: "agent".into(),
            status: crate::WorkflowV2Status::Accepted,
            result: None,
            error: None,
            failure_kind: None,
            item_input_hash: None,
            completion_evidence: Vec::new(),
        };
        store
            .save_branch_outcome(call_id, &outcome("same"))
            .unwrap();
        let legacy = root.join("branches/call_a");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("same.json"),
            serde_json::to_vec(&outcome("same")).unwrap(),
        )
        .unwrap();
        fs::write(
            legacy.join("older.json"),
            serde_json::to_vec(&outcome("older")).unwrap(),
        )
        .unwrap();

        assert_eq!(branch_count_for_call(root, call_id), 2);
    }
}
