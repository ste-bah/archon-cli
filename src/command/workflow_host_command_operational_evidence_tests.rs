use super::*;

fn fixture_with_secret(outcomes: Vec<Scripted>, secret: &str) -> Fixture {
    let mut fixture = fixture(outcomes);
    fixture
        .context
        .freeze_provider_environment
        .insert("ANTHROPIC_API_KEY".into(), secret.into());
    fixture.executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        fixture.context.clone(),
        fixture.store.run_dir(&fixture.run_id),
        fixture.process.clone(),
    );
    fixture
}

async fn recorded_stderr(stderr: String) -> String {
    let fixture = fixture(vec![
        Scripted::Stderr(EXIT_INCOMPLETE_RESUMABLE, "first attempt".into()),
        Scripted::Stderr(EXIT_INCOMPLETE_RESUMABLE, stderr),
    ]);
    let _ = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();
    let events = events(&fixture);
    let pauses = events_named(&events, "host_command_operational_pause");
    pauses[0]["detail"]["attempts"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["stderr_tail"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_pause_records_the_last_exit_and_redacted_stderr_tail() {
    let fixture = fixture_with_secret(
        vec![
            Scripted::Stderr(EXIT_INCOMPLETE_RESUMABLE, "first attempt detail".into()),
            Scripted::Stderr(
                EXIT_INCOMPLETE_RESUMABLE,
                "earlier diagnostic\nsecret-value-for-test\nlast diagnostic".into(),
            ),
        ],
        "secret-value-for-test",
    );
    let _ = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();

    let events = events(&fixture);
    let pauses = events_named(&events, "host_command_operational_pause");
    let attempt = pauses[0]["detail"]["attempts"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(attempt["exit_code"], EXIT_INCOMPLETE_RESUMABLE);
    let stderr = attempt["stderr_tail"].as_str().unwrap();
    assert!(stderr.contains("last diagnostic"), "{stderr}");
    assert!(!stderr.contains("secret-value-for-test"), "{stderr}");
    assert!(stderr.contains("[REDACTED]"), "{stderr}");
}

#[tokio::test]
async fn a_pause_redacts_an_api_key_shape_from_the_stored_record() {
    let value = "sk-ant-abcdefghijklmnopqrstuvwxyz0123456789";
    let stored = recorded_stderr(format!("provider rejected {value}")).await;
    assert!(!stored.contains(value), "{stored}");
}

#[tokio::test]
async fn a_pause_redacts_a_bearer_token_from_the_stored_record() {
    let value = "ya29.a0Af_abcDEF-123";
    let stored = recorded_stderr(format!("Authorization: Bearer {value}")).await;
    assert!(!stored.contains(value), "{stored}");
}

#[tokio::test]
async fn a_pause_redacts_a_key_value_credential_from_the_stored_record() {
    let value = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let stored = recorded_stderr(format!("GITHUB_TOKEN={value}")).await;
    assert!(!stored.contains(value), "{stored}");
}

#[tokio::test]
async fn a_pause_redacts_url_userinfo_credential_from_the_stored_record() {
    let value = "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789";
    let stored = recorded_stderr(format!(
        "failed to reach https://user:{value}@example.test/api"
    ))
    .await;
    assert!(!stored.contains(value), "{stored}");
}

#[tokio::test]
async fn a_pause_bounds_redacted_multibyte_stderr_to_a_valid_utf8_tail() {
    let stderr = format!("{}TAIL-END", "界".repeat(1800));
    let stored = recorded_stderr(stderr.clone()).await;
    assert!(stored.len() <= 4096, "stored {} bytes", stored.len());
    assert!(std::str::from_utf8(stored.as_bytes()).is_ok());
    assert!(stored.ends_with("TAIL-END"), "{stored}");
    assert!(
        stderr.ends_with(&stored),
        "stored output is not the stderr tail"
    );
}

#[tokio::test]
async fn a_missing_child_exit_status_is_stored_as_null() {
    let fixture = fixture(vec![Scripted::TimedOut(None)]);
    let _ = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();
    let events = events(&fixture);
    let pauses = events_named(&events, "host_command_operational_pause");
    let attempt = pauses[0]["detail"]["attempts"].as_array().unwrap()[0].clone();
    assert!(attempt["exit_code"].is_null(), "{attempt}");
}
