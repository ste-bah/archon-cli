use super::{
    digest_normalized, normalize_derived_hashes, normalize_embedded_receipt_for_test,
    normalize_publication_receipts, normalized_published_content, strip_timestamps,
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

#[test]
fn embedded_stdout_receipts_use_normalized_content_and_keep_content_changes() {
    let first_content = serde_json::json!({"ready": true, "path": "<project-root>/a"});
    let same_content = serde_json::json!({"path": "<project-root>/a", "ready": true});
    let changed_content = serde_json::json!({"ready": false, "path": "<project-root>/a"});
    let digest_one = "run-one-digest";
    let digest_two = "run-two-digest";
    let digest_changed = "changed-digest";
    let mut first = serde_json::json!({"stdout": format!(
        r#"{{"entries":[{{"blake3":"{digest_one}","relative_path":"acceptance-pin.json"}}]}}"#
    )});
    let mut same = serde_json::json!({"stdout": format!(
        r#"{{"entries":[{{"blake3":"{digest_two}","relative_path":"acceptance-pin.json"}}]}}"#
    )});
    let mut changed = serde_json::json!({"stdout": format!(
        r#"{{"entries":[{{"blake3":"{digest_changed}","relative_path":"acceptance-pin.json"}}]}}"#
    )});
    let map_one = [(digest_one.to_owned(), first_content)].into();
    let map_same = [(digest_two.to_owned(), same_content)].into();
    let map_changed = [(digest_changed.to_owned(), changed_content)].into();

    normalize_embedded_receipt_for_test(&mut first, &map_one);
    normalize_embedded_receipt_for_test(&mut same, &map_same);
    normalize_embedded_receipt_for_test(&mut changed, &map_changed);

    assert_eq!(first, same);
    assert_ne!(first, changed);
}

#[test]
fn build_fingerprint_is_normalized_but_logic_version_and_digest_are_kept() {
    let record = |build: &str| {
        normalized_record(serde_json::json!({
            "result": {"data": {"logicVersion": 9, "logicDigest": "pinned", "logicBuild": build}}
        }))
    };
    assert_eq!(record("build-one"), record("build-two"));
    let data = &record("build-one")["result"]["data"];
    assert_eq!(data["logicVersion"], 9);
    assert_eq!(data["logicDigest"], "pinned");
}

#[test]
fn only_interrupted_calls_lose_their_attempt_number() {
    let record = |id: &str, command: Option<&str>| {
        let mut value = serde_json::json!({
            "attempt": 2,
            "call": {"id": id, "method": "agent", "options": {"extra": {"commandId": command}}}
        });
        super::normalize_lifecycle_metadata(&mut value, &["skeleton-author-1", "freeze-skeleton"]);
        value
    };
    assert!(record("skeleton-author-1", None).get("attempt").is_none());
    assert!(
        record("other", Some("freeze-skeleton"))
            .get("attempt")
            .is_none()
    );
    assert_eq!(
        record("body-author-1", Some("land-task-body"))["attempt"],
        2
    );
}

#[test]
fn embedded_receipt_byte_len_of_normalized_content_is_dropped() {
    // Two machines publish the same envelope from roots of different lengths:
    // the raw byte lengths differ, the normalised content does not.
    let content = serde_json::json!({"report": "<project-root>/tasks/PRD-X"});
    let entry = |digest: &str, byte_len: u64| {
        serde_json::json!({"stdout": format!(
            r#"{{"entries":[{{"blake3":"{digest}","byte_len":{byte_len},"relative_path":"gate-envelope.json"}}]}}"#
        )})
    };
    let (mut short_root, mut long_root) = (entry("short", 40), entry("long", 90));
    normalize_embedded_receipt_for_test(
        &mut short_root,
        &[("short".to_owned(), content.clone())].into(),
    );
    normalize_embedded_receipt_for_test(&mut long_root, &[("long".to_owned(), content)].into());
    assert_eq!(short_root, long_root);
    assert!(!short_root["stdout"].as_str().unwrap().contains("byte_len"));
    // The normalised digest still tells content apart.
    let mut changed = entry("changed", 40);
    normalize_embedded_receipt_for_test(
        &mut changed,
        &[("changed".to_owned(), serde_json::json!({"report": "other"}))].into(),
    );
    assert_ne!(short_root, changed);
}

#[test]
fn a_manifest_followed_by_host_notes_is_still_normalised() {
    // land-task-body prints its manifest and then host notes (#428).
    let content = serde_json::json!({"report": "<project-root>/tasks/PRD-X"});
    let entry = |digest: &str| {
        serde_json::json!({"stdout": format!(
            "{{\"entries\":[{{\"blake3\":\"{digest}\",\"byte_len\":40,\"relative_path\":\"gate-envelope.json\"}}]}}\nHost note: replaced fields."
        )})
    };
    let (mut first, mut second) = (entry("first"), entry("second"));
    normalize_embedded_receipt_for_test(
        &mut first,
        &[("first".to_owned(), content.clone())].into(),
    );
    normalize_embedded_receipt_for_test(&mut second, &[("second".to_owned(), content)].into());
    assert_eq!(first, second);
    let stdout = first["stdout"].as_str().unwrap();
    assert!(
        stdout.ends_with("\nHost note: replaced fields."),
        "{stdout}"
    );
    assert!(!stdout.contains("byte_len"), "{stdout}");
}
