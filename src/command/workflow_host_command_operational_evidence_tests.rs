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
