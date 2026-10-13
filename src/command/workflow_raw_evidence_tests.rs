use super::*;

#[test]
fn persisted_parsed_service_account_masks_whole_object() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("service-account.json");
    let service_account = json!({
        "type": "service_account",
        "project_id": "private-project-canary",
        "private_key_id": "private-key-id-canary",
        "private_key": "private-key-canary",
        "client_email": "private-user@private-project-canary.iam.gserviceaccount.com"
    });

    save(&path, &service_account).unwrap();

    let saved = std::fs::read_to_string(path).unwrap();
    println!("saved file readback: {saved}");
    assert!(!saved.contains("private-project-canary"), "{saved}");
    assert!(
        !saved.contains("private-user@private-project-canary.iam.gserviceaccount.com"),
        "{saved}"
    );
}

#[test]
fn baseline_redaction_corpus_is_masked_by_every_evidence_sink() {
    // Expected before execution: every positive case copied from the
    // baseline redaction unit/smoke/transport tests is masked by the
    // observability, transport, workflow, and persisted raw-evidence paths.
    let positive = [
        concat!("sk", "-abcdefghijklmnopqrst0000"),
        concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
        concat!(
            "sk",
            "-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
        ),
        concat!(
            "sk",
            "-svcacct-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
        ),
        concat!("sk", "-ant-api03-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
        concat!("AK", "IAZZZZZZZZZZZZZZZZ"),
        concat!("AK", "IAIOSFODNN7EXAMPLE"),
        concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "u_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "s_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "r_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gi", "thub_pat_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("sk", "_live_abcdefghijklmnopqrstuvwx"),
        concat!("sk", "_test_abcdefghijklmnopqrstuvwx"),
        concat!("pk", "_live_abcdefghijklmnopqrstuvwx"),
        concat!("pk", "_test_abcdefghijklmnopqrstuvwx"),
        concat!("xo", "xb-1234567890-abcdef"),
        concat!("ey", "JhbGciOiJIUzI1NiIs.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
        "Bearer abc123",
        concat!("Bearer ya", "29.a0Af_abcDEF-123"),
        concat!(
            "{\"type\":\"service_account\",\"project_id\":\"my-project\",\"private_key_id\":\"abc123\",\"private_key\":\"--",
            "---BEGIN PRIVATE KEY-----\\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDexample\\n-----END PRIVATE KEY-----\\n\",\"client_email\":\"sa@my-project.iam.gserviceaccount.com\"}"
        ),
        "https://alice:pass123@example.test",
        concat!(
            "--",
            "---BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----"
        ),
        concat!(
            "--",
            "---BEGIN RSA PRIVATE KEY-----\\nabc\\n-----END RSA PRIVATE KEY-----"
        ),
        concat!(
            "--",
            "---BEGIN EC PRIVATE KEY-----\\nabc\\n-----END EC PRIVATE KEY-----"
        ),
        r#"{"access_token":"private-value","finish_reason":"max_tokens"}"#,
        "apikey=abc123",
        "api-key=abc123",
        "x-api-key: abc123",
        "password=abc123",
    ];
    let directory = tempfile::tempdir().unwrap();
    let v2 = directory.path().join("run").join("v2");
    for (index, input) in positive.into_iter().enumerate() {
        let observed = archon_observability::redaction::redact_secret_values(input);
        assert!(!observed.contains(input), "observability leaked {input:?}");
        let transported = archon_llm::redact_transport_evidence(input);
        assert!(!transported.contains(input), "transport leaked {input:?}");
        let workflow = archon_workflow::events::sanitize_value(json!(input));
        assert_ne!(workflow.as_str(), Some(input), "workflow leaked {input:?}");

        let call = format!("baseline-{index}");
        RawEvidence::start(&v2, &call, input).unwrap();
        let prompt_path = directory
            .path()
            .join("run/prompts")
            .join(format!("{call}.json"));
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(prompt_path).unwrap()).unwrap();
        assert_ne!(persisted["prompt"], input, "raw evidence leaked {input:?}");
    }

    // Structured keys use the baseline substring rules; key names are
    // data labels, so broad matching here does not corrupt prose.
    for key in [
        "api_key",
        "apiKey",
        "api-key",
        "access_token",
        "refresh_token",
        "client_secret",
        "auth_token",
        "provider_api_key",
        "x_auth_token",
        "password_hint",
        "reasoning_encrypted",
        "private_key_path",
        "request_cookie",
        "authorization",
        "cookie",
        "credentials",
        "oauth_token",
        "raw_text",
    ] {
        let value = json!({key:"AbCdEf0123456789GhIjKl4567890MnOpQrStUvWxYz"});
        let observed =
            archon_observability::shared_redactor::redact_json(value.clone(), "[REDACTED]");
        assert_ne!(
            observed[key], "AbCdEf0123456789GhIjKl4567890MnOpQrStUvWxYz",
            "observability key {key}"
        );
        let workflow = archon_workflow::events::sanitize_value(value.clone());
        if [
            "thinking",
            "reasoning",
            "reasoning_encrypted",
            "encrypted_reasoning",
            "oauth_token",
            "access_token",
            "refresh_token",
            "token",
            "api_key",
            "authorization",
            "password",
            "raw_text",
        ]
        .iter()
        .any(|field| key.contains(field))
        {
            assert!(workflow.get(key).is_none(), "workflow forbidden key {key}");
        } else {
            assert_ne!(
                workflow[key], "AbCdEf0123456789GhIjKl4567890MnOpQrStUvWxYz",
                "workflow credential key {key}"
            );
        }
        let transported = archon_llm::redact_transport_evidence(&value.to_string());
        assert!(
            !transported.contains("AbCdEf0123456789GhIjKl4567890MnOpQrStUvWxYz"),
            "transport key {key}"
        );
        let call = format!("baseline-key-{key}");
        RawEvidence::start(&v2, &call, &value.to_string()).unwrap();
        let path = directory
            .path()
            .join("run/prompts")
            .join(format!("{call}.json"));
        let persisted: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(
            !persisted["prompt"]
                .as_str()
                .unwrap()
                .contains("AbCdEf0123456789GhIjKl4567890MnOpQrStUvWxYz")
        );
    }

    let ordinary = [
        "token budget",
        "password reset flow",
        "the secret is nearby",
        "key=alpha",
        "$HOME $PATH ${VAR}",
    ];
    for (index, input) in ordinary.into_iter().enumerate() {
        assert_eq!(
            archon_observability::redaction::redact_secret_values(input),
            input
        );
        assert_eq!(archon_llm::redact_transport_evidence(input), input);
        assert_eq!(
            archon_workflow::events::sanitize_value(json!(input)),
            json!(input)
        );
        let call = format!("ordinary-{index}");
        RawEvidence::start(&v2, &call, input).unwrap();
        let path = directory
            .path()
            .join("run/prompts")
            .join(format!("{call}.json"));
        let persisted: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted["prompt"], input);
    }
}

#[test]
fn persisted_prompt_redacts_values_but_keeps_ordinary_language_and_shell_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let v2 = directory.path().join("run").join("v2");
    let ordinary = [
        "Secrets, API keys, authorization values, cookies, and session IDs must never be logged",
        r#"for token in alpha beta; do grep -qF -- "$token"; done"#,
        "The word secret appears in this prose, and token is a shell variable.",
    ]
    .join("\n");
    RawEvidence::start(&v2, "author-1", &ordinary).unwrap();
    let prompt_path = directory.path().join("run/prompts/author-1.json");
    let saved: Value = serde_json::from_slice(&std::fs::read(prompt_path).unwrap()).unwrap();
    assert_eq!(saved["prompt"].as_str(), Some(ordinary.as_str()));

    let credentials = [
        concat!("API_KEY=sk", "_test_123456789012345678901234"),
        concat!("Bearer gh", "p_123456789012345678901234567890123456"),
        "password=hunter2password",
        concat!("sk", "-proj-12345678901234567890"),
    ];
    let expected_secrets = [
        concat!("sk", "_test_123456789012345678901234"),
        concat!("gh", "p_123456789012345678901234567890123456"),
        "hunter2password",
        concat!("sk", "-proj-12345678901234567890"),
    ];
    for (index, (credential, secret)) in credentials.into_iter().zip(expected_secrets).enumerate() {
        let call = format!("author-secret-{index}");
        RawEvidence::start(&v2, &call, credential).unwrap();
        let path = directory
            .path()
            .join("run/prompts")
            .join(format!("{call}.json"));
        let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let prompt = saved["prompt"].as_str().unwrap();
        assert!(!prompt.contains(secret), "leaked {secret}: {prompt}");
        assert!(prompt.contains("<redacted>"), "missing marker: {prompt}");
    }

    let review_corpus = [
        ("Authorization: Bearer abc123", true),
        ("Proxy-Authorization: Basic abc123", true),
        ("x-api-key: abc123", true),
        ("https://alice:pass123@example.test", true),
        (r#"{"api_key":"abc123"}"#, true),
        (
            concat!(
                "--",
                "---BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"
            ),
            true,
        ),
        (concat!("sk", "-abcdefghijklmnopqrst0000"), true),
        (concat!("gh", "p_1234567890abcdef"), true),
        (concat!("AK", "IA0123456789ABCDEF"), true),
        (concat!("AS", "IA0123456789ABCDEF"), true),
        (concat!("xo", "xb-1234567890-abcdef"), true),
        (concat!("sk", "_live_abcdefghijklmnopqrstuvwx"), true),
        (concat!("sk", "_test_abcdefghijklmnopqrstuvwx"), true),
        (concat!("pk", "_live_abcdefghijklmnopqrstuvwx"), true),
        (concat!("pk", "_test_abcdefghijklmnopqrstuvwx"), true),
        (concat!("rk", "_live_abcdefghijklmnopqrstuvwx"), true),
        (
            concat!("ey", "JhbGciOiJIUzI1NiI.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
            true,
        ),
        ("Bearer authentication", true),
        ("key=alpha", false),
        ("token=\"$3\"", false),
        ("curl -fsS -H token=\"x\" http://localhost/health", false),
        ("$HOME $PATH", false),
        ("token budget", false),
        ("the secret is nearby", false),
        ("password reset flow", false),
    ];
    for (index, (input, should_mask)) in review_corpus.into_iter().enumerate() {
        let call = format!("author-review-{index}");
        RawEvidence::start(&v2, &call, input).unwrap();
        let path = directory
            .path()
            .join("run/prompts")
            .join(format!("{call}.json"));
        let raw = std::fs::read_to_string(path).unwrap();
        let saved: Value = serde_json::from_str(&raw).unwrap();
        let prompt = saved["prompt"].as_str().unwrap();
        assert_eq!(prompt == input, !should_mask, "{input:?} => {prompt:?}");
    }
}

/// Issue-245: the agent-output record is a public copy and stays redacted.
#[test]
fn the_public_agent_output_record_redacts_real_looking_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let v2 = temp.path().join("run").join("v2");
    let mut evidence = RawEvidence::start(&v2, "author-1", "prompt").unwrap();
    let outcome = WorkflowAgentOutcome {
            content: concat!("key sk", "-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 and Authorization: Bearer opaque-bearer-credential-123").into(),
            tool_uses: Vec::new(),
            tokens_in: Some(0),
            tokens_out: Some(0),
            stop_reason: Some("end_turn".into()),
        };
    evidence.finish(&Ok(outcome)).unwrap();
    let raw = std::fs::read_to_string(temp.path().join("run/agent-outputs/author-1.json")).unwrap();
    assert!(!raw.contains("sk-ant-"), "{raw}");
    assert!(!raw.contains("opaque-bearer-credential-123"), "{raw}");
}

#[test]
fn raw_outcome_omits_unreported_usage_and_keeps_reported_counts() {
    let absent = raw_outcome_result(
        WorkflowAgentOutcome {
            tokens_in: None,
            tokens_out: None,
            ..Default::default()
        },
        "end_turn".into(),
    );
    assert!(absent.data["tokensIn"].is_null());
    assert!(absent.data["tokensOut"].is_null());

    let reported = raw_outcome_result(
        WorkflowAgentOutcome {
            tokens_in: Some(123),
            tokens_out: Some(45),
            ..Default::default()
        },
        "end_turn".into(),
    );
    assert_eq!(reported.data["tokensIn"], json!(123));
    assert_eq!(reported.data["tokensOut"], json!(45));

    let reported_zero = raw_outcome_result(
        WorkflowAgentOutcome {
            tokens_in: Some(0),
            tokens_out: Some(0),
            ..Default::default()
        },
        "end_turn".into(),
    );
    assert_eq!(reported_zero.data["tokensIn"], json!(0));
    assert_eq!(reported_zero.data["tokensOut"], json!(0));
}

#[test]
fn fallback_masks_mixed_case_service_accounts_and_warns_with_file_path() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let cases = [
        json!({"TYPE":"SERVICE_ACCOUNT","project_id":"p","client_email":"e@x","zz_nested":{"ok":true}}),
        json!({"type":"Service_Account","project_id":"p","client_email":"e@x","zz_nested":{"ok":true}}),
    ];
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let layer = archon_observability::RedactionLayer::with_writer(SharedWriter(Arc::clone(&sink)));
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::TRACE)
        .with(layer);
    let paths = cases
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let path = directory.path().join(format!("evidence-{index}.json"));
            (path, value.clone())
        })
        .collect::<Vec<_>>();

    ::tracing::subscriber::with_default(subscriber, || {
        ::tracing::callsite::rebuild_interest_cache();
        for (path, value) in &paths {
            save(path, value).unwrap();
        }
    });

    for (path, _) in &paths {
        let saved = std::fs::read_to_string(path).unwrap();
        println!("saved file readback {}: {saved}", path.display());
        assert!(!saved.contains("\"p\""), "{}: {saved}", path.display());
        assert!(!saved.contains("e@x"), "{}: {saved}", path.display());
    }
    let captured = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
    println!("warning readback: {captured}");
    for (path, _) in &paths {
        assert!(captured.contains(&path.display().to_string()), "{captured}");
    }
    assert!(
        captured.contains("invalid JSON after the second pass"),
        "{captured}"
    );
    assert_eq!(
        captured
            .matches("raw evidence fallback redaction applied")
            .count(),
        2,
        "{captured}"
    );
}
