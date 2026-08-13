use super::*;

#[test]
fn validation_report_clean_native_is_deterministic() {
    let first = schema_artifact_value(&report(passed_checks())).unwrap();
    let second = schema_artifact_value(&report(passed_checks())).unwrap();
    let first_bytes = serde_json::to_vec_pretty(&first).unwrap();
    let second_bytes = serde_json::to_vec_pretty(&second).unwrap();
    let parsed: ValidationReport = serde_json::from_slice(&first_bytes).unwrap();

    assert_eq!(first_bytes, second_bytes);
    assert_eq!(parsed.normalized_sha256, "normalized-sha256");
    assert_eq!(
        parsed.content_sha256,
        ValidationReport::content_hash(&parsed.normalized_sha256, &parsed.checks, &parsed.summary)
    );
    assert!(parsed.is_consistent());
    assert!(crate::data_lake::validation_gate::allows_production(
        &parsed
    ));
}

#[test]
fn validation_rejects_required_jsonl_faults() {
    for (case, jsonl, expected_line) in malformed_jsonl_cases() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(format!("{case}.jsonl"));
        std::fs::write(&path, jsonl).unwrap();
        let evidence = format!("{:?}", read_jsonl_bars(&path).unwrap_err());
        assert!(
            evidence.contains("ohlcv.required_fields"),
            "{case}: {evidence}"
        );
        assert!(
            evidence.contains(&format!("line {expected_line}")),
            "{case}: {evidence}"
        );
        assert!(
            jsonl.trim().is_empty() || !evidence.contains(jsonl.trim()),
            "raw payload leaked: {evidence}"
        );
    }

    let invalid = OhlcvBar {
        timestamp: "2026-01-01T00:00:00Z".into(),
        open: 10.0,
        high: 9.0,
        low: 8.0,
        close: 10.0,
        volume: -1.0,
    };
    assert!(bad_ohlc_bar(&invalid));
    assert!(!volume_is_nonnegative(std::slice::from_ref(&invalid)));
    assert!(has_unsorted_timestamps(&[invalid.clone(), invalid]));
}

fn malformed_jsonl_cases() -> [(&'static str, &'static str, usize); 7] {
    [
        ("blank", "\n", 1),
        (
            "missing",
            "{\"timestamp\":\"2026-01-01T00:00:00Z\",\"open\":10,\"high\":11,\"low\":9,\"close\":10}\n",
            1,
        ),
        (
            "null",
            "{\"timestamp\":\"2026-01-01T00:00:00Z\",\"open\":null,\"high\":11,\"low\":9,\"close\":10,\"volume\":100}\n",
            1,
        ),
        (
            "duplicate",
            "{\"timestamp\":\"2026-01-01T00:00:00Z\",\"open\":10,\"open\":99,\"high\":11,\"low\":9,\"close\":10,\"volume\":100}\n",
            1,
        ),
        (
            "string encoded",
            "{\"timestamp\":\"2026-01-01T00:00:00Z\",\"open\":\"10\",\"high\":11,\"low\":9,\"close\":10,\"volume\":100}\n",
            1,
        ),
        (
            "wrong type on second line",
            "{\"timestamp\":\"2026-01-01T00:00:00Z\",\"open\":10,\"high\":11,\"low\":9,\"close\":10,\"volume\":100}\n{\"timestamp\":42,\"open\":10,\"high\":11,\"low\":9,\"close\":10,\"volume\":100}\n",
            2,
        ),
        ("malformed", "{not-json}\n", 1),
    ]
}
