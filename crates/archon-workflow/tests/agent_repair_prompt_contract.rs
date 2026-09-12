use archon_workflow::*;
use serde_json::{Value, json};

fn request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "repair-fixture".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions {
                extra: [(
                    "repository_audit_contract".into(),
                    json!({
                        "schema_version":1,"snapshot":"sealed-1","declared_paths":["src/new.rs"]
                    }),
                )]
                .into(),
                ..Default::default()
            },
        },
        role: "critic".into(),
        task: "Audit sealed source".into(),
        constraints: vec![],
        input: json!({}),
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: vec![],
        target_ownership_scopes: vec![],
    }
}
fn envelope(equivalents: Value) -> Value {
    json!({"status":"accepted","summary":"Audited source",
    "evidence":[{"kind":"inspection","summary":"x".repeat(15_000)}],
    "data":{"repository_audit":{"schema_version":1,"snapshot":"sealed-1","records":[{
        "declared_path":"src/new.rs","verdict":"exists_elsewhere","equivalents":equivalents,
        "required_action":"wire_or_migrate","reason":"Surrounding context: same behavior"
    }]}}})
}

#[test]
fn invalid_result_repair_preserves_full_json_and_locates_deep_value() {
    let adapter = WorkflowV2AgentAdapter::new();
    // Serialize explicitly: serde_json's default map ordering otherwise puts
    // `data` before evidence, accidentally keeping the fault in the head.
    let value = envelope(json!(["legacy/implementation.rs:123"]));
    let output = format!(
        "{{\"status\":\"accepted\",\"summary\":\"Audited source\",\"evidence\":{},\"data\":{}}}",
        value["evidence"], value["data"]
    );
    assert!(output.find("legacy/implementation.rs:123").unwrap() > 15_000);
    let error = adapter.parse_agent_output(&request(), &output).unwrap_err();
    assert!(matches!(error, WorkflowV2AgentError::InvalidResult(_)));
    let prompt = adapter.build_repair_prompt(&request(), &output, &error);
    assert!(prompt.contains(&output), "repair lost the prior answer");
    let fault = prompt
        .split("Fault context:")
        .nth(1)
        .expect("missing local window")
        .split("Previous JSON in full:")
        .next()
        .unwrap();
    assert!(fault.contains("legacy/implementation.rs:123"));
    assert!(fault.contains("Surrounding context: same behavior"));
    assert!(fault.contains("/data/repository_audit/records/0/equivalents/0"));
    assert!(!fault.contains(&"x".repeat(2_000)));
    assert!(prompt.contains("Return the same object with only the invalid fields corrected"));
}

#[test]
fn repair_locates_escaped_unicode_value_and_json_pointer_keys() {
    let output = format!(
        r#"{{"padding":"{}","a/b~c":["é\\bad\"path"]}}"#,
        "界".repeat(6_000)
    );
    let error = WorkflowV2AgentError::InvalidResult(format!("bad value: {:?}", "é\\bad\"path"));
    let prompt = WorkflowV2AgentAdapter::new().build_repair_prompt(&request(), &output, &error);
    let fault = prompt
        .split("Fault context:")
        .nth(1)
        .unwrap()
        .split("Previous JSON in full:")
        .next()
        .unwrap();
    assert!(fault.contains("/a~1b~0c/0"));
    assert!(fault.contains(r#"é\\bad\"path"#));
    assert!(fault.chars().count() < 2_000);
    assert!(prompt.contains(&output));
}

#[test]
fn non_json_repair_keeps_bounded_head_fallback() {
    let output = format!("{}TAIL_MUST_NOT_APPEAR", "界".repeat(3_000));
    let prompt = WorkflowV2AgentAdapter::new().build_repair_prompt(
        &request(),
        &output,
        &WorkflowV2AgentError::MalformedOutput("not JSON".into()),
    );
    assert!(prompt.contains(&"界".repeat(2_000)));
    assert!(!prompt.contains(&"界".repeat(2_001)));
    assert!(!prompt.contains("TAIL_MUST_NOT_APPEAR"));
    assert!(!prompt.contains("Previous JSON in full:"));
}

#[test]
fn unlocatable_validation_error_keeps_full_json_and_head_context() {
    let output = envelope(json!(["legacy/implementation.rs"])).to_string();
    let prompt = WorkflowV2AgentAdapter::new().build_repair_prompt(
        &request(),
        &output,
        &WorkflowV2AgentError::InvalidResult("unknown field combination".into()),
    );
    assert!(prompt.contains(&output));
    assert!(prompt.contains(&output.chars().take(2_000).collect::<String>()));
    assert!(prompt.contains("Return the same object with only the invalid fields corrected"));
}

#[test]
fn typed_deserialization_repair_preserves_valid_json() {
    let output = json!({"status":"invalid_status","summary":"x".repeat(15_000)}).to_string();
    let adapter = WorkflowV2AgentAdapter::new();
    let error = adapter.parse_agent_output(&request(), &output).unwrap_err();
    let prompt = adapter.build_repair_prompt(&request(), &output, &error);
    assert!(prompt.contains(&output));
    assert!(prompt.contains("Return the same object with only the invalid fields corrected"));
}

#[test]
fn equivalent_symbol_is_normalized_in_returned_result_before_filesystem_validation() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("legacy")).unwrap();
    std::fs::write(
        temp.path().join("legacy/implementation.rs"),
        "fn implementation() {}",
    )
    .unwrap();
    let mut request = request();
    request.repository_root = Some(temp.path().display().to_string());
    let output = envelope(json!(["legacy/implementation.rs:implementation"])).to_string();
    let adapter = WorkflowV2AgentAdapter::new();
    let result = adapter.parse_agent_output(&request, &output).unwrap();
    let record = &result.data["repository_audit"]["records"][0];
    assert_eq!(record["equivalents"], json!(["legacy/implementation.rs"]));
    let reason = record["reason"].as_str().unwrap();
    assert!(reason.starts_with("Surrounding context: same behavior"));
    assert!(reason.contains("legacy/implementation.rs:implementation"));
    // The accepted result is stable on revalidation, with no repeated annotation.
    let reparsed = adapter
        .parse_agent_output(&request, &serde_json::to_string(&result).unwrap())
        .unwrap();
    assert_eq!(reparsed.data, result.data);
}

#[test]
fn equivalent_normalization_does_not_relax_path_or_record_contracts() {
    let adapter = WorkflowV2AgentAdapter::new();
    for path in [
        "../escape.rs:valid",
        "/tmp/escape.rs:valid",
        "legacy/../escape.rs:valid",
        "legacy/.git/config:valid",
        "legacy\\file.rs:valid",
        "legacy/file.rs:123",
        "legacy/file.rs:bad-name",
        "legacy/file.rs:foo::bar",
        "legacy/file.rs:",
        "legacy/file.rs:foo()",
        "legacy/file.rs:valid\n",
        "legacy/file.rs:valid:other",
    ] {
        assert!(
            adapter
                .parse_agent_output(&request(), &envelope(json!([path])).to_string())
                .is_err(),
            "{path}"
        );
    }
    let mut output = envelope(json!(["legacy/file.rs:valid"]));
    output["data"]["repository_audit"]["records"][0]["declared_path"] = json!("src/new.rs:symbol");
    assert!(
        adapter
            .parse_agent_output(&request(), &output.to_string())
            .is_err()
    );
    for reason in ["".to_owned(), "x".repeat(2_048)] {
        let mut output = envelope(json!(["legacy/file.rs:valid"]));
        output["data"]["repository_audit"]["records"][0]["reason"] = json!(reason);
        assert!(
            adapter
                .parse_agent_output(&request(), &output.to_string())
                .is_err()
        );
    }
    assert!(
        adapter
            .parse_agent_output(
                &request(),
                &envelope(json!(["legacy/file.rs:valid", "legacy/file.rs"])).to_string()
            )
            .is_err()
    );
}

#[test]
fn equivalent_symbol_cannot_bypass_sealed_filesystem_confinement() {
    let temp = tempfile::tempdir().unwrap();
    let mut request = request();
    request.repository_root = Some(temp.path().display().to_string());
    let output = envelope(json!(["missing.rs:valid"])).to_string();
    assert!(
        WorkflowV2AgentAdapter::new()
            .parse_agent_output(&request, &output)
            .is_err()
    );
    #[cfg(unix)]
    {
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join("escape.rs")).unwrap();
        let output = envelope(json!(["escape.rs:valid"])).to_string();
        let error = WorkflowV2AgentAdapter::new()
            .parse_agent_output(&request, &output)
            .unwrap_err();
        assert!(error.to_string().contains("escapes sealed repository"));
    }
}

#[test]
fn repair_reports_every_named_invalid_value_with_its_path() {
    let output = json!({"first":"bad-one","nested":["bad-two"]}).to_string();
    let prompt = WorkflowV2AgentAdapter::new().build_repair_prompt(
        &request(),
        &output,
        &WorkflowV2AgentError::InvalidResult("invalid values: \"bad-one\", \"bad-two\"".into()),
    );
    assert!(prompt.contains("/first"));
    assert!(prompt.contains("/nested/0"));
    assert!(prompt.contains(&output));
}

#[test]
fn normalization_is_exclusive_to_audit_equivalents() {
    let mut request = request();
    request.call.options.extra.clear();
    let output = envelope(json!(["legacy/file.rs:symbol"]));
    let result = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output.to_string())
        .unwrap();
    assert_eq!(result.data, output["data"]);
    for path in ["src/new.rs:symbol", "../escape", "C:/file.rs:symbol"] {
        assert!(repository_audit::contract::validate_path(path).is_err());
    }
}

#[test]
fn tolerated_envelope_wrappers_keep_full_raw_reply_and_deep_fault_in_repair() {
    let adapter = WorkflowV2AgentAdapter::new();
    let value = envelope(json!(["legacy/implementation.rs:123"]));
    let document = format!(
        "{{\"status\":\"accepted\",\"summary\":\"Audited source\",\"evidence\":{},\"data\":{}}}",
        value["evidence"], value["data"]
    );
    for output in [
        format!("```json\n{document}\n```"),
        format!("The audit is complete.\n{document}\nEnd of audit."),
        format!("{},}}", document.strip_suffix('}').unwrap()),
    ] {
        assert!(output.find("legacy/implementation.rs:123").unwrap() > 15_000);
        let error = adapter.parse_agent_output(&request(), &output).unwrap_err();
        assert!(
            matches!(error, WorkflowV2AgentError::InvalidResult(_)),
            "{error}"
        );
        let prompt = adapter.build_repair_prompt(&request(), &output, &error);
        assert!(
            prompt.contains(&output),
            "repair discarded accepted wrapper's full raw text"
        );
        assert!(prompt.contains("Return the same object with only the invalid fields corrected"));
        let fault = prompt
            .split("Fault context:")
            .nth(1)
            .unwrap()
            .split("Previous JSON in full:")
            .next()
            .unwrap();
        assert!(fault.contains("/data/repository_audit/records/0/equivalents/0"));
        assert!(fault.contains("legacy/implementation.rs:123"));
        assert!(fault.contains("Surrounding context: same behavior"));
        assert!(!fault.contains(&"x".repeat(2_000)));
    }
}
