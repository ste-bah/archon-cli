#[tokio::test]
async fn response_header_retains_typed_retry_after() {
    let response = crate::connection_tests::error_response(503, "90").await;
    let wait = response
        .headers()
        .get("retry-after")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let status = response.status().as_u16();
    let error: crate::provider::LlmError =
        super::classify_error(status, &response.text().await.unwrap(), Some(&wait)).into();
    assert!(
        matches!(
            error,
            crate::provider::LlmError::Server {
                pipeline_retry_after_secs: Some(90),
                ..
            }
        ),
        "header lost: {error:?}"
    );
}
