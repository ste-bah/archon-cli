use super::{
    digest_normalized, normalize_derived_hashes, normalize_publication_receipts,
    normalized_published_content, strip_timestamps,
};

fn normalized_record(mut record: serde_json::Value) -> serde_json::Value {
    strip_timestamps(&mut record);
    normalize_derived_hashes(&mut record);
    record
}

#[test]
fn only_hashes_with_compared_preimages_are_recomputed() {
    let first = serde_json::json!({
        "evaluated_at": "2026-01-01T00:00:00Z",
        "call": {"input": {"prompt": "same"}, "id": "one"},
        "attempt": 1,
        "input_hash": "input-one",
        "output_hash": "output-one",
        "status": "accepted",
        "result": {"text": "same"},
        "unrelated": {"input_hash": "keep-one", "blake3": "opaque-one"},
        "verdict": "accepted",
        "task_text": "Create the required file",
        "findings": [{"message": "No finding"}]
    });
    let same_content = serde_json::json!({
        "evaluated_at": "2026-02-02T00:00:00Z",
        "call": {"input": {"prompt": "same"}, "id": "two"},
        "attempt": 1,
        "input_hash": "input-two",
        "output_hash": "output-two",
        "status": "accepted",
        "result": {"text": "same"},
        "unrelated": {"input_hash": "keep-two", "blake3": "opaque-two"},
        "verdict": "accepted",
        "task_text": "Create the required file",
        "findings": [{"message": "No finding"}]
    });
    let normalized_first = normalized_record(first.clone());
    let normalized_same = normalized_record(same_content);
    assert_eq!(
        normalized_first["input_hash"],
        normalized_same["input_hash"]
    );
    assert_eq!(
        normalized_first["output_hash"],
        normalized_same["output_hash"]
    );
    assert_eq!(normalized_first["unrelated"]["input_hash"], "keep-one");
    assert_eq!(normalized_first["unrelated"]["blake3"], "opaque-one");
    assert_eq!(normalized_first["result"], normalized_same["result"]);

    for changed in [
        serde_json::json!({"verdict": "rejected"}),
        serde_json::json!({"task_text": "Create a different file"}),
        serde_json::json!({"findings": [{"message": "A different finding"}]}),
    ] {
        let mut changed_record = first.clone();
        for (key, value) in changed.as_object().unwrap() {
            changed_record[key] = value.clone();
        }
        assert_ne!(
            normalized_record(first.clone()),
            normalized_record(changed_record)
        );
    }
}

#[test]
fn published_envelopes_ignore_root_paths_but_keep_findings() {
    let first = br#"{"project_root":"/tmp/one","findings":[{"rule":"task-set-lint","message":"missing task"}]}"#;
    let same_finding = br#"{"project_root":"/tmp/two","findings":[{"rule":"task-set-lint","message":"missing task"}]}"#;
    let different_finding = br#"{"project_root":"/tmp/two","findings":[{"rule":"task-set-lint","message":"different"}]}"#;
    let replacements_one = [("/tmp/one", "<project-root>")];
    let replacements_two = [("/tmp/two", "<project-root>")];

    assert_eq!(
        normalized_published_content(first, &replacements_one),
        normalized_published_content(same_finding, &replacements_two)
    );
    assert_ne!(
        normalized_published_content(first, &replacements_one),
        normalized_published_content(different_finding, &replacements_two)
    );
}

#[test]
fn receipt_digests_follow_compared_normalized_content_and_keep_unknown_prior_raw() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .join("published.json")
        .to_string_lossy()
        .into_owned();
    let bytes = br#"{"ready":true}"#;
    std::fs::write(&path, bytes).unwrap();
    let raw_digest = archon_workflow::task_set_contract::content_digest(bytes);
    let mut records = vec![
        (
            String::new(),
            serde_json::json!({"result":{"data":{"publicationReceipt":{"entries":[{
                "destination_path": path,
                "blake3": raw_digest,
                "prior_blake3": "prior-not-in-snapshot"
            }]}}}}),
        ),
        (
            String::new(),
            serde_json::json!({"result":{"data":{"publicationReceipt":{"entries":[{
                "destination_path": path,
                "blake3": raw_digest,
                "prior_blake3": raw_digest
            }]}}}}),
        ),
    ];

    normalize_publication_receipts(&mut records, &[]);

    let normalized_content = serde_json::json!({"ready": true});
    let expected = digest_normalized(&normalized_content);
    let first = &records[0].1["result"]["data"]["publicationReceipt"]["entries"][0];
    let second = &records[1].1["result"]["data"]["publicationReceipt"]["entries"][0];
    assert_eq!(first["blake3"], expected);
    assert_eq!(first["prior_blake3"], "prior-not-in-snapshot");
    assert_eq!(second["blake3"], expected);
    assert_eq!(second["prior_blake3"], expected);
}
